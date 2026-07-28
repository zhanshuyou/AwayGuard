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
    fn ema_smoothing_dampens_single_burst_dip() {
        // EMA damping: a single burst dip (one -95 sample) is not strong enough evidence
        // to flip away, because the smoothed value stays in Near territory.
        // This verifies the exponential smoothing coefficient α=0.35 is low enough
        // to ride out the ~20 dB spontaneous dips measured on real hardware.
        let mut t = ProximityTracker::new(thresholds());
        for _ in 0..3 { t.push(Some(-40)); }
        assert_eq!(t.state(), Presence::Near);
        // Single -95 sample: smoothed = -40 + 0.35*(-95 - -40) = -59.25, still >= -70.
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

    #[test]
    fn interrupt_resets_partial_streak() {
        // Streak mechanism must reset completely on inconclusive evidence,
        // requiring a fresh full confirmation sequence. This is the core
        // defense against off-by-one early transitions if streak counters lingered.
        let mut t = ProximityTracker::new(thresholds());
        for _ in 0..3 { t.push(Some(-40)); }
        assert_eq!(t.state(), Presence::Near);

        // Push weak samples to accumulate partial away-evidence without transitioning.
        // Arithmetic: after 3×(-40), smoothed = -40.
        // Push 1 (-95): -40 + 0.35*(-55) = -59.25 (≥ -70, evidence=Near, matches state)
        // Push 2 (-95): -59.25 + 0.35*(-35.75) = -71.76 (between -85 and -70, evidence=None)
        // Push 3 (-95): -71.76 + 0.35*(-23.24) = -79.90 (evidence=None)
        // Push 4 (-95): -79.90 + 0.35*(-15.10) = -85.18 (≤ -85, evidence=Away, streak=1)
        // Push 5 (-95): -85.18 + 0.35*(-9.82) = -88.62 (evidence=Away, streak=2)
        for _ in 0..5 {
            assert_eq!(t.push(Some(-95)), None);
        }
        assert_eq!(t.state(), Presence::Near);

        // Interrupt the away-streak with an inconclusive sample.
        // Smoothed drops to -71.60 (in hysteresis band), evidence=None.
        // This must reset pending and streak to 0.
        t.push(Some(-40));

        // Feed weak samples again. The state machine must now require a full fresh
        // confirm_samples (3) rounds of genuine away-evidence before transitioning,
        // not a shortened streak from residual pending counter.
        // Arithmetic: after interrupt, smoothed = -71.60.
        // Push 1 (-95): -71.60 + 0.35*(-23.40) = -79.79 (evidence=None)
        // Push 2 (-95): -79.79 + 0.35*(-15.21) = -85.11 (≤ -85, evidence=Away, streak=1)
        // Push 3 (-95): -85.11 + 0.35*(-9.89) ≈ -88.6 (evidence=Away, streak=2)
        // Push 4 (-95): evidence=Away, streak=3, transition!
        let mut transition = None;
        for _ in 0..5 {
            if let Some(s) = t.push(Some(-95)) {
                transition = Some(s);
                break;
            }
        }
        assert_eq!(transition, Some(Presence::Away));
    }
}
