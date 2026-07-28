// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/

pub mod ble;
pub mod commands;
pub mod config;
pub mod lock;
pub mod monitor;
pub mod proximity;

use crate::ble::{BleSource, ProximitySource};
use crate::config::Config;
use crate::lock::{MacScreenLocker, ScreenLocker};
use crate::monitor::{run_once, MonitorStatus};
use crate::proximity::{Presence, ProximityTracker, Thresholds};
use std::sync::{Arc, Mutex};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager};

pub struct AppState {
    pub source: Arc<dyn ProximitySource>,
    pub locker: Arc<dyn ScreenLocker>,
    pub config: Mutex<Config>,
    pub status: Mutex<MonitorStatus>,
    pub config_dir: std::path::PathBuf,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Menu bar app: no Dock icon.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let config_dir = app.path().app_config_dir()?;
            let config = Config::load(&config_dir);

            let source: Arc<dyn ProximitySource> = Arc::new(
                tauri::async_runtime::block_on(BleSource::new())
                    .map_err(std::io::Error::other)?,
            );
            let locker: Arc<dyn ScreenLocker> = Arc::new(MacScreenLocker::detect());

            let state = AppState {
                source: source.clone(),
                locker: locker.clone(),
                status: Mutex::new(MonitorStatus {
                    presence: Presence::Unknown,
                    rssi: None,
                    armed: config.armed,
                    error: None,
                }),
                config: Mutex::new(config),
                config_dir,
            };
            app.manage(state);

            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("AwayGuard")
                .on_tray_icon_event(|tray, _event| {
                    if let Some(w) = tray.app_handle().get_webview_window("main") {
                        let _ = if w.is_visible().unwrap_or(false) {
                            w.hide()
                        } else {
                            w.show().and_then(|_| w.set_focus())
                        };
                    }
                })
                .build(app)?;

            // Polling loop.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = handle.state::<AppState>();
                let mut tracker = {
                    let c = state.config.lock().unwrap();
                    ProximityTracker::new(Thresholds {
                        near_dbm: c.near_dbm,
                        away_dbm: c.away_dbm,
                        confirm_samples: c.confirm_samples,
                    })
                };
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    let (armed, target) = {
                        let c = state.config.lock().unwrap();
                        (c.armed, c.target_id.clone())
                    };
                    let Some(target) = target else { continue };
                    let status = run_once(
                        state.source.as_ref(),
                        &mut tracker,
                        state.locker.as_ref(),
                        armed,
                        &target,
                    )
                    .await;
                    *state.status.lock().unwrap() = status.clone();
                    let _ = handle.emit("status", status);
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_devices,
            commands::get_config,
            commands::set_config,
            commands::get_status,
            commands::lock_backend,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
