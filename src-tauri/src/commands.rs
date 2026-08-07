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

/// "Cancel — I'm still here": stop the countdown that is about to lock the
/// screen. Deliberately does NOT disarm. A cancelled countdown can only
/// restart from a fresh confirmed Near -> Away departure (see
/// `monitor::run_once`), so this dismisses exactly one pending lock and
/// leaves the guard in place for the next real departure.
#[tauri::command]
pub async fn cancel_pending_lock(state: State<'_, AppState>) -> Result<(), String> {
    state.grace.lock().await.cancel();
    Ok(())
}

/// The settings panes the two banners point at. An enum rather than a URL
/// parameter so the webview can only ever open these two known destinations,
/// never an arbitrary one.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SettingsPane {
    Bluetooth,
    LockScreen,
}

#[tauri::command]
pub fn open_settings(pane: SettingsPane) -> Result<(), String> {
    let url = match pane {
        SettingsPane::Bluetooth => "x-apple.systempreferences:com.apple.Bluetooth-Settings.extension",
        SettingsPane::LockScreen => "x-apple.systempreferences:com.apple.Lock-Screen-Settings.extension",
    };
    std::process::Command::new("/usr/bin/open")
        .arg(url)
        .status()
        .map_err(|e| format!("failed to open settings: {e}"))
        .and_then(|s| {
            if s.success() {
                Ok(())
            } else {
                Err("settings pane could not be opened".into())
            }
        })
}

#[tauri::command]
pub fn quit(app: tauri::AppHandle) {
    app.exit(0);
}
