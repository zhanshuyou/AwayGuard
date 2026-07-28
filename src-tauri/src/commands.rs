use crate::ble::DiscoveredDevice;
use crate::config::Config;
use crate::lock::LockBackend;
use crate::monitor::MonitorStatus;
use crate::AppState;
use tauri::State;

/// Minimum required gap between `near_dbm` and `away_dbm`. The proximity
/// state machine (`ProximityTracker::push`) checks the "near" branch first,
/// so if `near_dbm <= away_dbm` the tracker reports Near forever and the
/// app never locks -- silently, while still appearing armed. The UI's two
/// range sliders have overlapping legal ranges (-100..-50 for away,
/// -90..-30 for near) and can independently be dragged into that crossed
/// state, so this must be enforced here, not just in the UI.
const MIN_THRESHOLD_GAP_DBM: i16 = 1;

/// Enforce `near_dbm > away_dbm` on a config before it is persisted or
/// applied. If the incoming values are crossed or equal, `near_dbm` is
/// nudged up just above `away_dbm` -- the smallest change that restores a
/// non-empty hysteresis band without silently discarding the user's
/// intended `away_dbm`.
fn normalize_thresholds(mut config: Config) -> Config {
    if config.near_dbm <= config.away_dbm {
        config.near_dbm = config.away_dbm.saturating_add(MIN_THRESHOLD_GAP_DBM);
    }
    config
}

#[tauri::command]
pub async fn list_devices(state: State<'_, AppState>) -> Result<Vec<DiscoveredDevice>, String> {
    state.source.discover(4).await
}

#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> Config {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
pub fn set_config(state: State<'_, AppState>, config: Config) -> Result<(), String> {
    let config = normalize_thresholds(config);
    config.save(&state.config_dir)?;
    *state.config.lock().unwrap() = config;
    Ok(())
}

#[tauri::command]
pub fn get_status(state: State<'_, AppState>) -> MonitorStatus {
    state.status.lock().unwrap().clone()
}

#[tauri::command]
pub fn lock_backend(state: State<'_, AppState>) -> LockBackend {
    state.locker.backend()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(near_dbm: i16, away_dbm: i16) -> Config {
        Config {
            target_id: None,
            target_name: None,
            near_dbm,
            away_dbm,
            confirm_samples: 3,
            grace_seconds: 10,
            armed: false,
        }
    }

    #[test]
    fn normalize_thresholds_leaves_a_valid_gap_untouched() {
        let c = normalize_thresholds(config_with(-70, -85));
        assert_eq!((c.near_dbm, c.away_dbm), (-70, -85));
    }

    #[test]
    fn normalize_thresholds_fixes_crossed_values() {
        // near_dbm below away_dbm: the "never locks" fail-open case.
        let c = normalize_thresholds(config_with(-90, -60));
        assert!(c.near_dbm > c.away_dbm);
        assert_eq!(c.away_dbm, -60);
        assert_eq!(c.near_dbm, -59);
    }

    #[test]
    fn normalize_thresholds_fixes_equal_values() {
        let c = normalize_thresholds(config_with(-70, -70));
        assert!(c.near_dbm > c.away_dbm);
    }
}
