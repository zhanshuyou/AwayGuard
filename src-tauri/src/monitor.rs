use crate::ble::ProximitySource;
use crate::lock::ScreenLocker;
use crate::proximity::{Presence, ProximityTracker};
use serde::Serialize;
use std::time::Duration;

#[derive(Debug, Clone, Serialize)]
pub struct MonitorStatus {
    pub presence: Presence,
    pub rssi: Option<f32>,
    pub armed: bool,
    /// Set when the sensing chain itself is broken. While armed this must be
    /// surfaced, never swallowed — a silent failure looks identical to "safe".
    pub error: Option<String>,
    /// Whole seconds left before a pending departure fires the lock; `None`
    /// when nothing is counting down. The UI renders a live countdown from
    /// this, so it has to be the timer's real remaining time and not a
    /// frontend-side re-simulation of it.
    pub grace_remaining: Option<u64>,
    /// Seconds since the target last advertised; `None` until it has been
    /// seen at all. Distinguishes "never seen" from "seen, then vanished",
    /// which look identical if you only have `presence`.
    pub last_seen: Option<u64>,
    /// The state the tracker is accumulating evidence for and how many
    /// consecutive samples of it. Lets the UI show a transition that is
    /// underway before it is confirmed.
    pub pending: Option<Presence>,
    pub pending_samples: u8,
    /// Poll cadence in seconds, so the UI can say how fresh these numbers are
    /// instead of implying they are continuous.
    pub poll_interval: u64,
}

/// Assembles the status the UI sees from the monitor's own state, so every
/// number on screen comes from the thing that made the decision rather than
/// from a frontend guess. One place to build it also means the four exit
/// paths through `run_once` cannot drift apart in what they report.
fn snapshot(
    tracker: &ProximityTracker,
    grace: &GraceTimer,
    liveness: &Liveness,
    armed: bool,
    grace_seconds: u64,
    poll_interval: Duration,
    error: Option<String>,
) -> MonitorStatus {
    MonitorStatus {
        presence: tracker.state(),
        rssi: tracker.smoothed_rssi(),
        armed,
        error,
        grace_remaining: grace
            .remaining(Duration::from_secs(grace_seconds))
            // Round up: one second still on the clock must never read as 0,
            // which is the number the UI shows next to the word "Locking".
            .map(|d| d.as_secs() + u64::from(d.subsec_nanos() > 0)),
        last_seen: liveness.since_seen().map(|d| d.as_secs()),
        pending: tracker.pending(),
        pending_samples: tracker.streak(),
        poll_interval: poll_interval.as_secs(),
    }
}

/// How long it has been since the target actually advertised.
///
/// Deliberately pure, for the same reason `GraceTimer` is: the caller threads
/// its own per-round tick in, so "the phone vanished four seconds ago" is
/// testable without sleeping. Not seeing the peripheral and the adapter
/// erroring are both misses — in either case we have no fresh evidence, and
/// the UI must not imply we do.
#[derive(Debug, Default)]
pub struct Liveness {
    /// `None` until the target has been seen even once, so a device that was
    /// picked but never found reads as "never seen" rather than "seen 0s ago".
    since_seen: Option<Duration>,
}

impl Liveness {
    pub fn new() -> Self {
        Self { since_seen: None }
    }

    /// The target advertised this round.
    pub fn seen(&mut self) {
        self.since_seen = Some(Duration::ZERO);
    }

    /// The target did not advertise this round (or we could not look).
    pub fn missed(&mut self, tick: Duration) {
        if let Some(d) = &mut self.since_seen {
            *d += tick;
        }
    }

    pub fn since_seen(&self) -> Option<Duration> {
        self.since_seen
    }
}

/// Tracks the grace period between a confirmed Near -> Away departure and
/// the lock actually firing, so the user has a window to return before the
/// screen locks. Deliberately pure (no `Instant::now()`): callers thread
/// their own elapsed-time-per-round in, which is what makes this unit
/// testable without real sleeping.
#[derive(Debug, Default)]
pub struct GraceTimer {
    /// `Some(accumulated)` while a departure is pending confirmation-of-lock;
    /// `None` when there is nothing counting down.
    accumulated: Option<Duration>,
}

impl GraceTimer {
    pub fn new() -> Self {
        Self { accumulated: None }
    }

    pub fn is_pending(&self) -> bool {
        self.accumulated.is_some()
    }

    /// Begin (or restart) the countdown from zero.
    pub fn start(&mut self) {
        self.accumulated = Some(Duration::ZERO);
    }

    /// Stop the countdown. Used when the phone returns to Near before the
    /// grace period elapses -- the pending lock must not fire.
    ///
    /// Also what the user's "I'm still here" button calls. Cancelling does
    /// not disarm: the countdown can only start again from a fresh confirmed
    /// Near -> Away departure (see `run_once`), so dismissing this one lock
    /// never silently dismisses the next one.
    pub fn cancel(&mut self) {
        self.accumulated = None;
    }

    /// Time left before a pending countdown fires; `None` when nothing is
    /// pending. Saturating, so a countdown that has already run past `grace`
    /// (it fires on the next `advance`) reports zero rather than underflowing.
    pub fn remaining(&self, grace: Duration) -> Option<Duration> {
        self.accumulated.map(|acc| grace.saturating_sub(acc))
    }

    /// Advance a pending countdown by `tick`. Returns `true` exactly once,
    /// the round the accumulated time reaches `grace` -- the caller should
    /// fire the delayed action then. A no-op (returns `false`) when nothing
    /// is pending.
    pub fn advance(&mut self, tick: Duration, grace: Duration) -> bool {
        match &mut self.accumulated {
            None => false,
            Some(acc) => {
                *acc += tick;
                if *acc >= grace {
                    self.accumulated = None;
                    true
                } else {
                    false
                }
            }
        }
    }
}

/// One polling round. A lock may only fire following a departure from a
/// *confirmed Near* state -- Unknown -> Away (e.g. at launch, before the
/// phone has appeared in the scan cache) must never lock, only Near -> Away.
/// Once that departure is confirmed, `grace_seconds` must elapse (tracked in
/// `grace`, advanced by `elapsed` each round) before the lock actually
/// fires, and returning to Near during that window cancels it.
#[allow(clippy::too_many_arguments)]
pub async fn run_once(
    source: &dyn ProximitySource,
    tracker: &mut ProximityTracker,
    grace: &mut GraceTimer,
    liveness: &mut Liveness,
    locker: &dyn ScreenLocker,
    armed: bool,
    target_id: &str,
    grace_seconds: u64,
    elapsed: Duration,
) -> MonitorStatus {
    let prev_state = tracker.state();

    // Disarming must immediately cancel any pending countdown, not merely
    // freeze it. It used to only be gated out of *starting*/*advancing* via
    // `armed &&` below, while cancellation only checked presence -- so a
    // countdown pending at the moment of disarming just sat there frozen,
    // and resumed (and could fire) the instant the user re-armed, even
    // though the phone never re-established a confirmed Near baseline in
    // between. This check must run before the early-return-on-error path
    // below too, so a fault while disarmed still cancels it.
    if !armed {
        grace.cancel();
    }

    let (sample, error) = match source.sample(target_id).await {
        Ok(v) => (v, None),
        // A sensor error is NOT evidence of departure -- never feed it to
        // the state machine, and never let it START a countdown (that only
        // happens below, on a real confirmed transition). But once a
        // departure is already confirmed and its grace countdown pending,
        // the fault must not stall it forever: the user is away, and a
        // countdown that can never complete because the adapter keeps
        // erroring is as bad as never confirming the departure at all. So a
        // pending countdown still advances (and can still fire) through a
        // run of errors; the fault is surfaced in `error` either way.
        Err(e) => {
            // A fault is not a sighting: the staleness clock keeps running so
            // the UI cannot show a reassuringly recent "last seen" while the
            // adapter is actually broken.
            liveness.missed(elapsed);
            let mut error = Some(e);
            if armed && grace.advance(elapsed, Duration::from_secs(grace_seconds)) {
                if let Err(le) = locker.lock() {
                    error = Some(format!("lock failed: {le}"));
                }
            }
            return snapshot(
                tracker,
                grace,
                liveness,
                armed,
                grace_seconds,
                elapsed,
                error,
            );
        }
    };

    match sample {
        Some(_) => liveness.seen(),
        None => liveness.missed(elapsed),
    }

    let transition = tracker.push(sample);

    // The phone came back: whatever countdown was pending must not fire.
    if tracker.state() == Presence::Near {
        grace.cancel();
    }

    // Only a confirmed departure FROM Near starts the countdown. This is
    // what keeps Unknown -> Away (no confirmed Near baseline yet) from ever
    // reaching the locker.
    if armed && prev_state == Presence::Near && transition == Some(Presence::Away) {
        grace.start();
    }

    if armed && grace.advance(elapsed, Duration::from_secs(grace_seconds)) {
        if let Err(e) = locker.lock() {
            return snapshot(
                tracker,
                grace,
                liveness,
                armed,
                grace_seconds,
                elapsed,
                Some(format!("lock failed: {e}")),
            );
        }
    }

    snapshot(tracker, grace, liveness, armed, grace_seconds, elapsed, error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ble::{DiscoveredDevice, FakeSource};
    use crate::lock::{LockBackend, ScreenLocker};
    use crate::proximity::{Presence, ProximityTracker, Thresholds};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RecordingLocker {
        calls: AtomicUsize,
    }
    impl RecordingLocker {
        fn new() -> Self { Self { calls: AtomicUsize::new(0) } }
        fn calls(&self) -> usize { self.calls.load(Ordering::SeqCst) }
    }
    impl ScreenLocker for RecordingLocker {
        fn lock(&self) -> Result<(), String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn backend(&self) -> LockBackend { LockBackend::PrivateApi }
    }

    /// A source that always errors, used to exercise the sensor-fault path
    /// (LEDGER 1): `FakeSource` can only ever return `Ok`, so it cannot
    /// cover `run_once`'s early-return-on-`Err` branch.
    struct ErrorSource;
    #[async_trait::async_trait]
    impl ProximitySource for ErrorSource {
        async fn discover(&self, _secs: u64) -> Result<Vec<DiscoveredDevice>, String> {
            Ok(vec![])
        }
        async fn sample(&self, _target_id: &str) -> Result<Option<i16>, String> {
            Err("sensor fault".into())
        }
    }

    fn tracker() -> ProximityTracker {
        ProximityTracker::new(Thresholds { near_dbm: -70, away_dbm: -85, confirm_samples: 3 })
    }

    // grace_seconds=0 + elapsed=ZERO reproduces the pre-grace-period
    // behavior (fire immediately on a confirmed departure) so these tests
    // keep testing exactly what they tested before grace existed.
    const NO_GRACE: u64 = 0;
    const NO_TICK: Duration = Duration::ZERO;

    #[tokio::test]
    async fn locks_once_on_departure() {
        // With confirm_samples=3 and the EMA smoothing (alpha=0.35), the
        // smoothed value takes 6 consecutive -100 samples to cross away_dbm
        // (-85) three times in a row after starting from a Near baseline of
        // -40 (verified against ProximityTracker's actual transition point).
        // FakeSource repeats the last sample once exhausted, so the extra
        // loop rounds below exercise "stays away" with the same -100 value.
        let source = FakeSource::new(vec![
            Some(-40), Some(-40), Some(-40),                             // establish Near
            Some(-100), Some(-100), Some(-100), Some(-100), Some(-100), Some(-100), // depart
        ]);
        let locker = RecordingLocker::new();
        let mut t = tracker();
        let mut grace = GraceTimer::new();
        let mut seen = Liveness::new();
        for _ in 0..12 {
            run_once(&source, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", NO_GRACE, NO_TICK).await;
        }
        assert_eq!(locker.calls(), 1, "departure must lock exactly once");
    }

    #[tokio::test]
    async fn never_locks_while_disarmed() {
        // Same fixture as locks_once_on_departure: 6 consecutive -100 samples
        // are required from a -40 baseline to actually cross the Near -> Away
        // transition (see that test's comment for the verified arithmetic).
        // Using a fixture that never reaches the transition would let this
        // test pass even with the `armed &&` guard deleted from run_once, so
        // we drive a real departure and then assert it happened.
        let source = FakeSource::new(vec![
            Some(-40), Some(-40), Some(-40),                             // establish Near
            Some(-100), Some(-100), Some(-100), Some(-100), Some(-100), Some(-100), // depart
        ]);
        let locker = RecordingLocker::new();
        let mut t = tracker();
        let mut grace = GraceTimer::new();
        let mut seen = Liveness::new();
        let mut last = None;
        for _ in 0..12 {
            last = Some(run_once(&source, &mut t, &mut grace, &mut seen, &locker, false, "fake-device", NO_GRACE, NO_TICK).await);
        }
        assert_eq!(last.unwrap().presence, Presence::Away, "fixture must actually reach departure");
        assert_eq!(locker.calls(), 0, "disarmed must never lock, even on a real departure");
    }

    #[tokio::test]
    async fn brief_dip_does_not_lock() {
        let source = FakeSource::new(vec![
            Some(-40), Some(-40), Some(-40),
            Some(-95),            // one spurious dip
            Some(-40), Some(-40),
        ]);
        let locker = RecordingLocker::new();
        let mut t = tracker();
        let mut grace = GraceTimer::new();
        let mut seen = Liveness::new();
        for _ in 0..6 {
            run_once(&source, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", NO_GRACE, NO_TICK).await;
        }
        assert_eq!(locker.calls(), 0);
    }

    #[tokio::test]
    async fn reports_presence_in_status() {
        let source = FakeSource::new(vec![Some(-40), Some(-40), Some(-40)]);
        let locker = RecordingLocker::new();
        let mut t = tracker();
        let mut grace = GraceTimer::new();
        let mut seen = Liveness::new();
        let mut last = None;
        for _ in 0..3 {
            last = Some(run_once(&source, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", NO_GRACE, NO_TICK).await);
        }
        assert_eq!(last.unwrap().presence, Presence::Near);
    }

    #[tokio::test]
    async fn unknown_to_away_does_not_lock() {
        // CRITICAL 2 regression test. At launch the tracker starts Unknown
        // and the scan cache is empty, so a real device produces several
        // rounds of Ok(None) before its first advertisement is seen. That
        // is an Unknown -> Away transition, not a Near -> Away departure,
        // and must never lock -- otherwise the app locks ~6s after launch
        // whenever the phone hasn't been discovered yet.
        let source = FakeSource::new(vec![None, None, None, None, None, None]);
        let locker = RecordingLocker::new();
        let mut t = tracker();
        let mut grace = GraceTimer::new();
        let mut seen = Liveness::new();
        let mut last = None;
        for _ in 0..6 {
            last = Some(run_once(&source, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", NO_GRACE, NO_TICK).await);
        }
        assert_eq!(last.unwrap().presence, Presence::Away, "fixture must actually reach Away from Unknown");
        assert_eq!(locker.calls(), 0, "Unknown -> Away must never lock; only a confirmed Near -> Away departure may");
    }

    #[tokio::test]
    async fn near_to_away_still_locks() {
        // Companion to unknown_to_away_does_not_lock: a departure from an
        // actually-confirmed Near state must still lock (with no grace
        // period configured here, immediately on confirmation).
        let source = FakeSource::new(vec![
            Some(-40), Some(-40), Some(-40),
            Some(-100), Some(-100), Some(-100), Some(-100), Some(-100), Some(-100),
        ]);
        let locker = RecordingLocker::new();
        let mut t = tracker();
        let mut grace = GraceTimer::new();
        let mut seen = Liveness::new();
        for _ in 0..12 {
            run_once(&source, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", NO_GRACE, NO_TICK).await;
        }
        assert_eq!(locker.calls(), 1, "a departure from a confirmed Near state must lock");
    }

    #[tokio::test]
    async fn lock_fires_only_after_the_grace_period_elapses() {
        // CRITICAL 3. A confirmed departure must not lock immediately: the
        // configured grace period must accumulate first.
        //
        // Same fixture as locks_once_on_departure: confirm_samples=3 with
        // alpha=0.35 EMA smoothing needs 6 consecutive -100 samples (rounds
        // 4-9) to cross away_dbm three times in a row from a -40 baseline
        // -- the transition confirms on round 9, not round 6.
        let source = FakeSource::new(vec![
            Some(-40), Some(-40), Some(-40),
            Some(-100), Some(-100), Some(-100), Some(-100), Some(-100), Some(-100),
        ]);
        let locker = RecordingLocker::new();
        let mut t = tracker();
        let mut grace = GraceTimer::new();
        let mut seen = Liveness::new();
        let grace_seconds = 10;
        let tick = Duration::from_secs(2);

        // Rounds 1-9: establish Near, then confirm the departure. The
        // confirming round itself starts the countdown and contributes its
        // own tick (2s of the 10s grace).
        for _ in 0..9 {
            run_once(&source, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", grace_seconds, tick).await;
        }
        assert_eq!(locker.calls(), 0, "lock must not fire immediately on confirmed departure");

        // 3 more rounds bring accumulated grace time to 2 + 2*3 = 8s, still
        // short of the 10s grace period.
        for _ in 0..3 {
            run_once(&source, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", grace_seconds, tick).await;
        }
        assert_eq!(locker.calls(), 0, "grace period has not fully elapsed yet");

        // One more round crosses the 10s grace period.
        run_once(&source, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", grace_seconds, tick).await;
        assert_eq!(locker.calls(), 1, "lock must fire once the grace period elapses");
    }

    #[tokio::test]
    async fn returning_to_near_during_grace_cancels_the_pending_lock() {
        // CRITICAL 3. If the phone comes back before the grace period
        // elapses, the pending lock must be cancelled, not merely delayed.
        // grace_seconds is set very large here so the test cannot pass by
        // accident (i.e. the countdown racing to completion before Near
        // reconfirms) -- it isolates cancellation from timing.
        let depart = FakeSource::new(vec![
            Some(-40), Some(-40), Some(-40),
            Some(-100), Some(-100), Some(-100), Some(-100), Some(-100), Some(-100),
        ]);
        let locker = RecordingLocker::new();
        let mut t = tracker();
        let mut grace = GraceTimer::new();
        let mut seen = Liveness::new();
        let grace_seconds = 1_000;
        let tick = Duration::from_secs(2);

        for _ in 0..12 {
            run_once(&depart, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", grace_seconds, tick).await;
        }
        assert_eq!(t.state(), Presence::Away, "fixture must actually confirm departure");
        assert!(grace.is_pending(), "a confirmed departure must start the grace countdown");

        // Phone returns: feed strong samples until Near reconfirms.
        let ret = FakeSource::new(vec![Some(-40)]); // FakeSource repeats the last value
        for _ in 0..12 {
            run_once(&ret, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", grace_seconds, tick).await;
            if t.state() == Presence::Near {
                break;
            }
        }
        assert_eq!(t.state(), Presence::Near, "fixture must actually reconfirm Near");
        assert!(!grace.is_pending(), "returning to Near must cancel the pending grace lock");
        assert_eq!(locker.calls(), 0, "a cancelled grace countdown must never fire the lock");
    }

    #[tokio::test]
    async fn sensor_error_does_not_advance_streak_or_lock() {
        // LEDGER 1(b). A sensor fault (Err) must not be fed to the state
        // machine as evidence of departure, and must surface in
        // MonitorStatus.error rather than being swallowed.
        let near_source = FakeSource::new(vec![Some(-40), Some(-40), Some(-40)]);
        let locker = RecordingLocker::new();
        let mut t = tracker();
        let mut grace = GraceTimer::new();
        let mut seen = Liveness::new();
        for _ in 0..3 {
            run_once(&near_source, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", NO_GRACE, NO_TICK).await;
        }
        assert_eq!(t.state(), Presence::Near, "fixture must actually establish Near first");

        let error_source = ErrorSource;
        let mut last = None;
        for _ in 0..5 {
            last = Some(run_once(&error_source, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", NO_GRACE, NO_TICK).await);
        }

        assert_eq!(t.state(), Presence::Near, "a sensor fault must not be fed to the state machine as evidence of departure");
        assert_eq!(locker.calls(), 0, "a sensor fault must never lock the screen");
        assert_eq!(
            last.unwrap().error,
            Some("sensor fault".to_string()),
            "the fault must reach MonitorStatus.error, not be swallowed"
        );
    }

    #[tokio::test]
    async fn disarm_cancels_pending_countdown_and_rearm_does_not_resume_it() {
        // CRITICAL regression (final review round): grace.start()/advance()
        // were gated on `armed`, but cancellation only checked presence --
        // so a pending countdown neither advanced nor cancelled while
        // disarmed, it just froze. Re-arming while still Away resumed the
        // stale countdown and locked without ever re-confirming Near,
        // defeating the Unknown -> Away guard's entire purpose.
        let source = FakeSource::new(vec![
            Some(-40), Some(-40), Some(-40),
            Some(-100), Some(-100), Some(-100), Some(-100), Some(-100), Some(-100),
        ]);
        let locker = RecordingLocker::new();
        let mut t = tracker();
        let mut grace = GraceTimer::new();
        let mut seen = Liveness::new();
        let grace_seconds = 4; // small enough that a resumed countdown would fire immediately
        let tick = Duration::from_secs(2);

        // Rounds 1-9: establish Near, then confirm the departure (armed).
        for _ in 0..9 {
            run_once(&source, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", grace_seconds, tick).await;
        }
        assert_eq!(t.state(), Presence::Away, "fixture must actually confirm departure");
        assert!(grace.is_pending(), "a confirmed departure must start the grace countdown");
        assert_eq!(locker.calls(), 0, "grace period has not elapsed yet");

        // User disarms while still away. Many disarmed rounds pass -- the
        // countdown must never fire while disarmed, and must actually be
        // cancelled (not just frozen).
        for _ in 0..50 {
            run_once(&source, &mut t, &mut grace, &mut seen, &locker, false, "fake-device", grace_seconds, tick).await;
        }
        assert_eq!(locker.calls(), 0, "must never lock while disarmed");
        assert!(!grace.is_pending(), "disarming must cancel the pending countdown, not just freeze it");

        // User re-arms while the phone is still away -- no fresh Near
        // baseline was ever re-established in between. This must NOT
        // resume the stale countdown and lock: that would fire with the
        // user at the keyboard, having never left since re-arming.
        for _ in 0..10 {
            run_once(&source, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", grace_seconds, tick).await;
        }
        assert_eq!(
            locker.calls(), 0,
            "re-arming while already away must not lock without a fresh confirmed Near -> Away departure"
        );
    }

    #[tokio::test]
    async fn sensor_fault_during_pending_grace_still_advances_and_locks() {
        // IMPORTANT regression (final review round): the early return on
        // Err used to skip grace.advance() entirely, so a fault that began
        // right after a confirmed departure and persisted would stall the
        // countdown forever -- the user is away, the error is surfaced,
        // but the screen never locks. A pending countdown must still
        // complete on schedule even if every subsequent poll faults.
        let confirm_source = FakeSource::new(vec![
            Some(-40), Some(-40), Some(-40),
            Some(-100), Some(-100), Some(-100), Some(-100), Some(-100), Some(-100),
        ]);
        let locker = RecordingLocker::new();
        let mut t = tracker();
        let mut grace = GraceTimer::new();
        let mut seen = Liveness::new();
        let grace_seconds = 4;
        let tick = Duration::from_secs(2);

        for _ in 0..9 {
            run_once(&confirm_source, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", grace_seconds, tick).await;
        }
        assert!(grace.is_pending(), "a confirmed departure must start the grace countdown");
        assert_eq!(locker.calls(), 0, "2s of 4s accumulated -- not due yet");

        // The adapter now faults on every poll. The pending countdown must
        // still complete (2s already accumulated + this round's 2s = 4s).
        let error_source = ErrorSource;
        let status = run_once(&error_source, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", grace_seconds, tick).await;

        assert_eq!(locker.calls(), 1, "a pending grace countdown must still complete and lock even while the sensor is erroring");
        assert_eq!(
            status.error,
            Some("sensor fault".to_string()),
            "the fault must still be surfaced even though the lock fired"
        );
    }

    #[tokio::test]
    async fn status_reports_the_countdown_the_timer_is_actually_running() {
        // The popover renders "Locking in Ns" straight from grace_remaining.
        // If that number were re-simulated in the frontend it could disagree
        // with the timer that will actually fire -- the one number on screen
        // that absolutely has to be the real one.
        let source = FakeSource::new(vec![
            Some(-40), Some(-40), Some(-40),
            Some(-100), Some(-100), Some(-100), Some(-100), Some(-100), Some(-100),
        ]);
        let locker = RecordingLocker::new();
        let mut t = tracker();
        let mut grace = GraceTimer::new();
        let mut seen = Liveness::new();
        let grace_seconds = 10;
        let tick = Duration::from_secs(2);

        let mut status = None;
        for _ in 0..9 {
            status = Some(
                run_once(&source, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", grace_seconds, tick).await,
            );
        }
        // Round 9 confirms the departure and contributes its own 2s tick.
        assert_eq!(status.as_ref().unwrap().grace_remaining, Some(8));

        let next = run_once(&source, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", grace_seconds, tick).await;
        assert_eq!(next.grace_remaining, Some(6), "the countdown must tick down with the poll loop");

        for _ in 0..3 {
            status = Some(
                run_once(&source, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", grace_seconds, tick).await,
            );
        }
        assert_eq!(locker.calls(), 1, "fixture must actually reach the lock");
        assert_eq!(
            status.unwrap().grace_remaining,
            None,
            "once the lock has fired there is no countdown left to show"
        );
    }

    #[tokio::test]
    async fn never_seen_and_vanished_are_different_statuses() {
        // Both read as Presence::Away, but "we have never found this phone"
        // and "we saw it 6 seconds ago" are very different claims, and the
        // popover makes exactly that distinction.
        let locker = RecordingLocker::new();
        let mut t = tracker();
        let mut grace = GraceTimer::new();
        let mut seen = Liveness::new();
        let tick = Duration::from_secs(2);

        // Never seen: the peripheral has not advertised at all.
        let absent = FakeSource::new(vec![None]); // FakeSource repeats the last value
        let status = run_once(&absent, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", 10, tick).await;
        assert_eq!(status.last_seen, None, "a phone that never appeared has no last-seen time");

        // Seen once, then gone.
        let present = FakeSource::new(vec![Some(-40)]);
        let status = run_once(&present, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", 10, tick).await;
        assert_eq!(status.last_seen, Some(0));

        let mut status = None;
        for _ in 0..3 {
            status = Some(
                run_once(&absent, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", 10, tick).await,
            );
        }
        assert_eq!(status.unwrap().last_seen, Some(6), "staleness must accrue once the phone stops advertising");
    }

    #[tokio::test]
    async fn a_sensor_fault_does_not_count_as_a_sighting() {
        // The staleness clock must keep running through a fault. Freezing it
        // would let the popover show a reassuringly recent "seen 2s ago"
        // during exactly the window where we can see nothing at all.
        let locker = RecordingLocker::new();
        let mut t = tracker();
        let mut grace = GraceTimer::new();
        let mut seen = Liveness::new();
        let tick = Duration::from_secs(2);

        let present = FakeSource::new(vec![Some(-40)]);
        run_once(&present, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", 10, tick).await;

        let broken = ErrorSource;
        let status = run_once(&broken, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", 10, tick).await;
        assert_eq!(status.last_seen, Some(2), "a fault is not a sighting");
        assert!(status.error.is_some());
    }

    #[tokio::test]
    async fn cancelling_a_countdown_needs_a_fresh_departure_to_start_another() {
        // What "Cancel — I'm still here" does. It must dismiss exactly one
        // pending lock: staying away afterwards must not silently restart
        // the countdown, and it must not disarm either -- the next real
        // Near -> Away departure still has to lock.
        let depart = FakeSource::new(vec![
            Some(-40), Some(-40), Some(-40),
            Some(-100), Some(-100), Some(-100), Some(-100), Some(-100), Some(-100),
        ]);
        let locker = RecordingLocker::new();
        let mut t = tracker();
        let mut grace = GraceTimer::new();
        let mut seen = Liveness::new();
        let grace_seconds = 10;
        let tick = Duration::from_secs(2);

        for _ in 0..9 {
            run_once(&depart, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", grace_seconds, tick).await;
        }
        assert!(grace.is_pending(), "fixture must actually start a countdown");

        // The user says they are still here.
        grace.cancel();

        // Staying away must not resurrect it, however many rounds pass.
        let still_away = FakeSource::new(vec![Some(-100)]);
        for _ in 0..20 {
            run_once(&still_away, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", grace_seconds, tick).await;
        }
        assert_eq!(t.state(), Presence::Away, "the phone really is still out of range");
        assert_eq!(locker.calls(), 0, "a cancelled countdown must not restart on its own");

        // But the guard is still up: come back, leave again, and it locks.
        let back = FakeSource::new(vec![Some(-40)]);
        for _ in 0..12 {
            run_once(&back, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", grace_seconds, tick).await;
            if t.state() == Presence::Near {
                break;
            }
        }
        assert_eq!(t.state(), Presence::Near, "fixture must actually reconfirm Near");

        let leave = FakeSource::new(vec![Some(-100)]);
        for _ in 0..20 {
            run_once(&leave, &mut t, &mut grace, &mut seen, &locker, true, "fake-device", grace_seconds, tick).await;
        }
        assert_eq!(locker.calls(), 1, "cancelling one lock must not disarm the next departure");
    }

    #[test]
    fn grace_timer_starts_not_pending() {
        let g = GraceTimer::new();
        assert!(!g.is_pending());
    }

    #[test]
    fn grace_timer_reports_no_remaining_time_when_nothing_is_pending() {
        let g = GraceTimer::new();
        assert_eq!(g.remaining(Duration::from_secs(10)), None);
    }

    #[test]
    fn grace_timer_remaining_saturates_instead_of_underflowing() {
        // `advance` clears the timer the round it reaches `grace`, so an
        // overrun is only reachable if the grace period is shortened under a
        // countdown that is already past it. Report zero, not a wrapped
        // enormous number that would render as "Locking in 18446744073s".
        let mut g = GraceTimer::new();
        g.start();
        g.advance(Duration::from_secs(8), Duration::from_secs(30));
        assert_eq!(g.remaining(Duration::from_secs(4)), Some(Duration::ZERO));
    }

    #[test]
    fn grace_timer_fires_exactly_once_accumulated_time_reaches_grace() {
        let mut g = GraceTimer::new();
        g.start();
        assert!(!g.advance(Duration::from_secs(4), Duration::from_secs(10)));
        assert!(!g.advance(Duration::from_secs(4), Duration::from_secs(10)));
        assert!(g.advance(Duration::from_secs(4), Duration::from_secs(10)), "12s accumulated must fire a 10s grace");
        assert!(!g.is_pending(), "firing must clear the pending state so it cannot fire twice");
    }

    #[test]
    fn grace_timer_cancel_prevents_a_later_fire() {
        let mut g = GraceTimer::new();
        g.start();
        g.cancel();
        assert!(!g.is_pending());
        assert!(!g.advance(Duration::from_secs(100), Duration::from_secs(1)), "a cancelled timer must not fire even if advanced well past the grace period");
    }
}
