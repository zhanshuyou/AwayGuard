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
    // In-memory config is updated first and unconditionally, so it stays
    // authoritative even if the on-disk save fails (read-only/full dir,
    // permissions, ...). Previously `config.save(...)?` ran first: a save
    // error meant the polling loop kept using the OLD in-memory config
    // while the frontend believed the new one had taken effect.
    *state.config.lock().unwrap() = config.clone();
    config.save(&state.config_dir)
}

#[tauri::command]
pub fn get_status(state: State<'_, AppState>) -> MonitorStatus {
    state.status.lock().unwrap().clone()
}

#[tauri::command]
pub fn lock_backend(state: State<'_, AppState>) -> LockBackend {
    state.locker.backend()
}
