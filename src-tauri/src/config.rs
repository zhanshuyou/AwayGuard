use serde::{Deserialize, Serialize};
use std::path::Path;

const FILE_NAME: &str = "config.json";

/// Minimum required gap between `near_dbm` and `away_dbm`. The proximity
/// state machine (`ProximityTracker::push`) checks the "near" branch
/// first, so if `near_dbm <= away_dbm` the tracker reports Near forever
/// and the app never locks -- silently, while still appearing armed.
///
/// This must be a genuinely usable hysteresis band, not just a non-empty
/// one: measured RSSI spread on real hardware is ~20 dB, so a band as
/// narrow as 1 dB still yields conclusive evidence on effectively every
/// poll and can lock the screen from three unlucky samples at the desk.
const MIN_THRESHOLD_GAP_DBM: i16 = 12;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub target_id: Option<String>,
    pub target_name: Option<String>,
    pub near_dbm: i16,
    pub away_dbm: i16,
    pub confirm_samples: u8,
    pub grace_seconds: u64,
    pub armed: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            target_id: None,
            target_name: None,
            // Measured: ~-35 dBm at the desk, so -70/-85 leaves a wide
            // hysteresis band to absorb the ~20 dB spontaneous dips.
            near_dbm: -70,
            away_dbm: -85,
            confirm_samples: 3,
            grace_seconds: 10,
            armed: false,
        }
    }
}

impl Config {
    /// Enforce a `near_dbm - away_dbm >= MIN_THRESHOLD_GAP_DBM` floor in
    /// place. If the gap is crossed, equal, or simply too narrow to be a
    /// usable hysteresis band, `near_dbm` is nudged up to exactly
    /// `away_dbm + MIN_THRESHOLD_GAP_DBM` -- the smallest change that
    /// restores a genuinely usable band without silently discarding the
    /// caller's intended `away_dbm`.
    ///
    /// This is a floor check (`< MIN_THRESHOLD_GAP_DBM`), not just a
    /// crossing check (`near_dbm <= away_dbm`): an uncrossed but narrow
    /// pair like near=-70/away=-71 is still a 1 dB band against ~20 dB of
    /// measured RSSI noise, and previously passed through untouched
    /// because it never crossed.
    ///
    /// This lives on `Config` itself (rather than only in the `set_config`
    /// command) so it is impossible to bypass: it also runs inside
    /// `load()`, which is the only other place a `Config` enters the app
    /// (from disk, written by an older build, hand-edited, or otherwise
    /// already crossed).
    pub fn normalize_thresholds(&mut self) {
        if self.near_dbm.saturating_sub(self.away_dbm) < MIN_THRESHOLD_GAP_DBM {
            self.near_dbm = self.away_dbm.saturating_add(MIN_THRESHOLD_GAP_DBM);
        }
    }

    pub fn load(dir: &Path) -> Config {
        // Any failure falls back to defaults, which are disarmed.
        // Never inherit a half-parsed armed state.
        let mut config: Config = std::fs::read_to_string(dir.join(FILE_NAME))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        config.normalize_thresholds();
        config
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(FILE_NAME), json).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_disarmed_with_no_target() {
        let c = Config::default();
        assert!(!c.armed);
        assert_eq!(c.target_id, None);
        // Hysteresis band must be non-empty or the state machine flaps.
        assert!(c.near_dbm > c.away_dbm);
    }

    #[test]
    fn load_returns_defaults_when_file_is_missing() {
        let dir = std::env::temp_dir().join("awayguard-test-missing");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!Config::load(&dir).armed);
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = std::env::temp_dir().join("awayguard-test-roundtrip");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let c = Config {
            target_id: Some("18b1c44c-6313-7e59-ccfb-4aa35e765c2d".into()),
            armed: true,
            grace_seconds: 12,
            ..Config::default()
        };
        c.save(&dir).unwrap();

        let back = Config::load(&dir);
        assert_eq!(back.target_id.as_deref(), Some("18b1c44c-6313-7e59-ccfb-4aa35e765c2d"));
        assert!(back.armed);
        assert_eq!(back.grace_seconds, 12);
    }

    #[test]
    fn load_falls_back_to_defaults_on_corrupt_file() {
        let dir = std::env::temp_dir().join("awayguard-test-corrupt");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.json"), b"{ not json").unwrap();
        // Must not panic, and must not silently arm.
        assert!(!Config::load(&dir).armed);
    }

    #[test]
    fn normalize_thresholds_leaves_a_valid_gap_untouched() {
        let mut c = Config { near_dbm: -70, away_dbm: -85, ..Config::default() };
        c.normalize_thresholds();
        assert_eq!((c.near_dbm, c.away_dbm), (-70, -85));
    }

    #[test]
    fn normalize_thresholds_fixes_crossed_values() {
        // near_dbm below away_dbm: the "never locks" fail-open case.
        let mut c = Config { near_dbm: -90, away_dbm: -60, ..Config::default() };
        c.normalize_thresholds();
        assert!(c.near_dbm > c.away_dbm);
        assert_eq!(c.away_dbm, -60);
        assert_eq!(c.near_dbm, -60 + MIN_THRESHOLD_GAP_DBM);
    }

    #[test]
    fn normalize_thresholds_enforces_a_floor_even_when_not_crossed() {
        // near_dbm > away_dbm here (never crosses), but the gap is only
        // 1 dB -- against ~20 dB of measured RSSI noise, that reaches the
        // running tracker as conclusive evidence on effectively every
        // poll. The old crossing-only check (`near_dbm <= away_dbm`)
        // let this straight through.
        let mut c = Config { near_dbm: -70, away_dbm: -71, ..Config::default() };
        c.normalize_thresholds();
        assert_eq!(c.away_dbm, -71, "away_dbm must not be silently discarded");
        assert_eq!(c.near_dbm, -71 + MIN_THRESHOLD_GAP_DBM, "gap must be widened to the full floor");
    }

    #[test]
    fn normalize_thresholds_fixes_equal_values() {
        let mut c = Config { near_dbm: -70, away_dbm: -70, ..Config::default() };
        c.normalize_thresholds();
        assert!(c.near_dbm > c.away_dbm);
    }

    #[test]
    fn load_normalizes_a_crossed_config_found_on_disk() {
        // Simulates a config written by an older build, or hand-edited,
        // that predates the near/away gap invariant. load() must not
        // hand a crossed config to the running app even though the JSON
        // itself is well-formed.
        let dir = std::env::temp_dir().join("awayguard-test-crossed-on-disk");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let crossed = Config { near_dbm: -90, away_dbm: -60, ..Config::default() };
        std::fs::write(
            dir.join("config.json"),
            serde_json::to_string(&crossed).unwrap(),
        )
        .unwrap();

        let loaded = Config::load(&dir);
        assert!(
            loaded.near_dbm > loaded.away_dbm,
            "load() must normalize a crossed config already on disk"
        );
    }
}
