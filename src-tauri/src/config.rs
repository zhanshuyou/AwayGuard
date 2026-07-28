use serde::{Deserialize, Serialize};
use std::path::Path;

const FILE_NAME: &str = "config.json";

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
    pub fn load(dir: &Path) -> Config {
        // Any failure falls back to defaults, which are disarmed.
        // Never inherit a half-parsed armed state.
        std::fs::read_to_string(dir.join(FILE_NAME))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
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
        assert_eq!(Config::load(&dir).armed, false);
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = std::env::temp_dir().join("awayguard-test-roundtrip");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut c = Config::default();
        c.target_id = Some("18b1c44c-6313-7e59-ccfb-4aa35e765c2d".into());
        c.armed = true;
        c.grace_seconds = 12;
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
}
