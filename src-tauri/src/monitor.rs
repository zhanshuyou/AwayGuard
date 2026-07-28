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
    pub fn cancel(&mut self) {
        self.accumulated = None;
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
    locker: &dyn ScreenLocker,
    armed: bool,
    target_id: &str,
    grace_seconds: u64,
    elapsed: Duration,
) -> MonitorStatus {
    let prev_state = tracker.state();

    let (sample, error) = match source.sample(target_id).await {
        Ok(v) => (v, None),
        // A sensor error is NOT evidence of departure. Report it and do not
        // feed the state machine, so a broken adapter cannot lock the machine.
        // The pending grace countdown (if any) is left untouched: a sensor
        // hiccup is not a reason to cancel a departure that was already
        // confirmed, nor to advance it.
        Err(e) => {
            return MonitorStatus {
                presence: tracker.state(),
                rssi: tracker.smoothed_rssi(),
                armed,
                error: Some(e),
            }
        }
    };

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
            return MonitorStatus {
                presence: tracker.state(),
                rssi: tracker.smoothed_rssi(),
                armed,
                error: Some(format!("lock failed: {e}")),
            };
        }
    }

    MonitorStatus {
        presence: tracker.state(),
        rssi: tracker.smoothed_rssi(),
        armed,
        error,
    }
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
        for _ in 0..12 {
            run_once(&source, &mut t, &mut grace, &locker, true, "fake-device", NO_GRACE, NO_TICK).await;
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
        let mut last = None;
        for _ in 0..12 {
            last = Some(run_once(&source, &mut t, &mut grace, &locker, false, "fake-device", NO_GRACE, NO_TICK).await);
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
        for _ in 0..6 {
            run_once(&source, &mut t, &mut grace, &locker, true, "fake-device", NO_GRACE, NO_TICK).await;
        }
        assert_eq!(locker.calls(), 0);
    }

    #[tokio::test]
    async fn reports_presence_in_status() {
        let source = FakeSource::new(vec![Some(-40), Some(-40), Some(-40)]);
        let locker = RecordingLocker::new();
        let mut t = tracker();
        let mut grace = GraceTimer::new();
        let mut last = None;
        for _ in 0..3 {
            last = Some(run_once(&source, &mut t, &mut grace, &locker, true, "fake-device", NO_GRACE, NO_TICK).await);
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
        let mut last = None;
        for _ in 0..6 {
            last = Some(run_once(&source, &mut t, &mut grace, &locker, true, "fake-device", NO_GRACE, NO_TICK).await);
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
        for _ in 0..12 {
            run_once(&source, &mut t, &mut grace, &locker, true, "fake-device", NO_GRACE, NO_TICK).await;
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
        let grace_seconds = 10;
        let tick = Duration::from_secs(2);

        // Rounds 1-9: establish Near, then confirm the departure. The
        // confirming round itself starts the countdown and contributes its
        // own tick (2s of the 10s grace).
        for _ in 0..9 {
            run_once(&source, &mut t, &mut grace, &locker, true, "fake-device", grace_seconds, tick).await;
        }
        assert_eq!(locker.calls(), 0, "lock must not fire immediately on confirmed departure");

        // 3 more rounds bring accumulated grace time to 2 + 2*3 = 8s, still
        // short of the 10s grace period.
        for _ in 0..3 {
            run_once(&source, &mut t, &mut grace, &locker, true, "fake-device", grace_seconds, tick).await;
        }
        assert_eq!(locker.calls(), 0, "grace period has not fully elapsed yet");

        // One more round crosses the 10s grace period.
        run_once(&source, &mut t, &mut grace, &locker, true, "fake-device", grace_seconds, tick).await;
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
        let grace_seconds = 1_000;
        let tick = Duration::from_secs(2);

        for _ in 0..12 {
            run_once(&depart, &mut t, &mut grace, &locker, true, "fake-device", grace_seconds, tick).await;
        }
        assert_eq!(t.state(), Presence::Away, "fixture must actually confirm departure");
        assert!(grace.is_pending(), "a confirmed departure must start the grace countdown");

        // Phone returns: feed strong samples until Near reconfirms.
        let ret = FakeSource::new(vec![Some(-40)]); // FakeSource repeats the last value
        for _ in 0..12 {
            run_once(&ret, &mut t, &mut grace, &locker, true, "fake-device", grace_seconds, tick).await;
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
        for _ in 0..3 {
            run_once(&near_source, &mut t, &mut grace, &locker, true, "fake-device", NO_GRACE, NO_TICK).await;
        }
        assert_eq!(t.state(), Presence::Near, "fixture must actually establish Near first");

        let error_source = ErrorSource;
        let mut last = None;
        for _ in 0..5 {
            last = Some(run_once(&error_source, &mut t, &mut grace, &locker, true, "fake-device", NO_GRACE, NO_TICK).await);
        }

        assert_eq!(t.state(), Presence::Near, "a sensor fault must not be fed to the state machine as evidence of departure");
        assert_eq!(locker.calls(), 0, "a sensor fault must never lock the screen");
        assert_eq!(
            last.unwrap().error,
            Some("sensor fault".to_string()),
            "the fault must reach MonitorStatus.error, not be swallowed"
        );
    }

    #[test]
    fn grace_timer_starts_not_pending() {
        let g = GraceTimer::new();
        assert!(!g.is_pending());
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
