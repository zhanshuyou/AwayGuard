use serde::Serialize;

/// Exponential smoothing factor. Low enough to ride out the ~20 dB
/// spontaneous dips measured on real hardware.
const SMOOTHING_ALPHA: f32 = 0.35;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Presence {
    Unknown,
    Near,
    Away,
}

#[derive(Debug, Clone, Copy)]
pub struct Thresholds {
    /// Smoothed RSSI at or above this counts as evidence of Near.
    pub near_dbm: i16,
    /// Smoothed RSSI at or below this counts as evidence of Away.
    pub away_dbm: i16,
    /// Consecutive rounds of consistent evidence required to change state.
    pub confirm_samples: u8,
}

pub struct ProximityTracker {
    thresholds: Thresholds,
    smoothed: Option<f32>,
    state: Presence,
    pending: Option<Presence>,
    streak: u8,
}

impl ProximityTracker {
    pub fn new(thresholds: Thresholds) -> Self {
        Self { thresholds, smoothed: None, state: Presence::Unknown, pending: None, streak: 0 }
    }

    pub fn state(&self) -> Presence {
        self.state
    }

    pub fn smoothed_rssi(&self) -> Option<f32> {
        self.smoothed
    }

    /// Feed one scan round. `None` means the peripheral was not seen at all.
    /// Returns `Some(new_state)` only on an actual transition.
    pub fn push(&mut self, sample: Option<i16>) -> Option<Presence> {
        let evidence = match sample {
            // Not seen at all is strong evidence of departure, and must not
            // pollute the smoothed value with a fabricated reading.
            None => Some(Presence::Away),
            Some(raw) => {
                let next = match self.smoothed {
                    Some(prev) => prev + SMOOTHING_ALPHA * (raw as f32 - prev),
                    None => raw as f32,
                };
                self.smoothed = Some(next);

                if next >= self.thresholds.near_dbm as f32 {
                    Some(Presence::Near)
                } else if next <= self.thresholds.away_dbm as f32 {
                    Some(Presence::Away)
                } else {
                    // Inside the hysteresis band: no evidence either way.
                    None
                }
            }
        };

        match evidence {
            Some(e) if e != self.state => {
                if self.pending == Some(e) {
                    self.streak = self.streak.saturating_add(1);
                } else {
                    self.pending = Some(e);
                    self.streak = 1;
                }
                if self.streak >= self.thresholds.confirm_samples {
                    self.state = e;
                    self.pending = None;
                    self.streak = 0;
                    return Some(e);
                }
                None
            }
            _ => {
                // Evidence agrees with current state, or is inconclusive.
                self.pending = None;
                self.streak = 0;
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thresholds() -> Thresholds {
        Thresholds { near_dbm: -70, away_dbm: -85, confirm_samples: 3 }
    }

    #[test]
    fn starts_unknown() {
        let t = ProximityTracker::new(thresholds());
        assert_eq!(t.state(), Presence::Unknown);
    }

    #[test]
    fn confirms_near_after_enough_strong_samples() {
        let mut t = ProximityTracker::new(thresholds());
        assert_eq!(t.push(Some(-40)), None);
        assert_eq!(t.push(Some(-40)), None);
        assert_eq!(t.push(Some(-40)), Some(Presence::Near));
        assert_eq!(t.state(), Presence::Near);
    }

    #[test]
    fn single_weak_sample_does_not_flip_to_away() {
        // The core anti-flap guarantee: measured RSSI dips ~20 dB spontaneously.
        let mut t = ProximityTracker::new(thresholds());
        for _ in 0..3 { t.push(Some(-40)); }
        assert_eq!(t.state(), Presence::Near);
        assert_eq!(t.push(Some(-95)), None);
        assert_eq!(t.state(), Presence::Near);
    }

    #[test]
    fn confirms_away_after_sustained_weak_samples() {
        let mut t = ProximityTracker::new(thresholds());
        for _ in 0..3 { t.push(Some(-40)); }
        let mut transition = None;
        for _ in 0..12 {
            if let Some(s) = t.push(Some(-100)) { transition = Some(s); break; }
        }
        assert_eq!(transition, Some(Presence::Away));
    }

    #[test]
    fn missing_sample_counts_as_away_evidence() {
        // None means the peripheral was not seen in that scan round,
        // which is different from a weak reading.
        let mut t = ProximityTracker::new(thresholds());
        for _ in 0..3 { t.push(Some(-40)); }
        let mut transition = None;
        for _ in 0..12 {
            if let Some(s) = t.push(None) { transition = Some(s); break; }
        }
        assert_eq!(transition, Some(Presence::Away));
    }

    #[test]
    fn samples_inside_hysteresis_band_hold_current_state() {
        let mut t = ProximityTracker::new(thresholds());
        for _ in 0..3 { t.push(Some(-40)); }
        for _ in 0..10 {
            assert_eq!(t.push(Some(-78)), None); // between away_dbm and near_dbm
        }
        assert_eq!(t.state(), Presence::Near);
    }

    #[test]
    fn returns_transition_only_once_per_change() {
        let mut t = ProximityTracker::new(thresholds());
        t.push(Some(-40));
        t.push(Some(-40));
        assert_eq!(t.push(Some(-40)), Some(Presence::Near));
        assert_eq!(t.push(Some(-40)), None);
        assert_eq!(t.push(Some(-40)), None);
    }
}
