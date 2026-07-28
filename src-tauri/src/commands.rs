use crate::ble::DiscoveredDevice;
use crate::config::Config;
use crate::lock::LockBackend;
use crate::monitor::MonitorStatus;
use crate::AppState;
use tauri::State;

#[tauri::command]
pub async fn list_devices(state: State<'_, AppState>) -> Result<Vec<DiscoveredDevice>, String> {
    state.source.discover(4).await
}

#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> Config {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
pub fn set_config(state: State<'_, AppState>, mut config: Config) -> Result<(), String> {
    // Fail-open guard: see Config::normalize_thresholds. Also enforced
    // inside Config::load(), so a crossed config can never reach the
    // running app regardless of how it got onto disk.
    config.normalize_thresholds();
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
