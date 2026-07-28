use crate::ble::ProximitySource;
use crate::lock::ScreenLocker;
use crate::proximity::{Presence, ProximityTracker};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct MonitorStatus {
    pub presence: Presence,
    pub rssi: Option<f32>,
    pub armed: bool,
    /// Set when the sensing chain itself is broken. While armed this must be
    /// surfaced, never swallowed — a silent failure looks identical to "safe".
    pub error: Option<String>,
}

/// One polling round. Locks only on the Near -> Away transition, so a phone
/// that stays away does not re-lock on every subsequent round.
pub async fn run_once(
    source: &dyn ProximitySource,
    tracker: &mut ProximityTracker,
    locker: &dyn ScreenLocker,
    armed: bool,
    target_id: &str,
) -> MonitorStatus {
    let (sample, error) = match source.sample(target_id).await {
        Ok(v) => (v, None),
        // A sensor error is NOT evidence of departure. Report it and do not
        // feed the state machine, so a broken adapter cannot lock the machine.
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

    if armed && transition == Some(Presence::Away) {
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
    use crate::ble::FakeSource;
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

    fn tracker() -> ProximityTracker {
        ProximityTracker::new(Thresholds { near_dbm: -70, away_dbm: -85, confirm_samples: 3 })
    }

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
        for _ in 0..12 {
            run_once(&source, &mut t, &locker, true, "fake-device").await;
        }
        assert_eq!(locker.calls(), 1, "departure must lock exactly once");
    }

    #[tokio::test]
    async fn never_locks_while_disarmed() {
        let source = FakeSource::new(vec![Some(-40), Some(-100), Some(-100), Some(-100)]);
        let locker = RecordingLocker::new();
        let mut t = tracker();
        for _ in 0..6 {
            run_once(&source, &mut t, &locker, false, "fake-device").await;
        }
        assert_eq!(locker.calls(), 0);
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
        for _ in 0..6 {
            run_once(&source, &mut t, &locker, true, "fake-device").await;
        }
        assert_eq!(locker.calls(), 0);
    }

    #[tokio::test]
    async fn reports_presence_in_status() {
        let source = FakeSource::new(vec![Some(-40), Some(-40), Some(-40)]);
        let locker = RecordingLocker::new();
        let mut t = tracker();
        let mut last = None;
        for _ in 0..3 {
            last = Some(run_once(&source, &mut t, &locker, true, "fake-device").await);
        }
        assert_eq!(last.unwrap().presence, Presence::Near);
    }
}
