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
use crate::monitor::{run_once, GraceTimer, MonitorStatus};
use crate::proximity::{Presence, ProximityTracker, Thresholds};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{Emitter, Manager};

/// The polling loop's cadence. Used both as the `sleep` between rounds and
/// as the `elapsed` tick fed to the grace-period countdown, so the grace
/// timer's accumulated time actually tracks wall-clock time.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

pub struct AppState {
    pub source: Arc<dyn ProximitySource>,
    pub locker: Arc<dyn ScreenLocker>,
    pub config: Mutex<Config>,
    pub status: Mutex<MonitorStatus>,
    pub config_dir: std::path::PathBuf,
    /// Kept so the polling loop can push presence/armed/error into the
    /// tray tooltip -- for a menu bar app whose popover is hidden by
    /// default, the tray is the only surface that's always visible.
    pub tray: TrayIcon<tauri::Wry>,
}

/// One-line tray tooltip summarizing the current monitoring state. An error
/// takes priority over everything else, since it means the sensing chain
/// itself may be broken -- exactly what must never be silently invisible.
fn tray_tooltip(status: &MonitorStatus) -> String {
    if let Some(e) = &status.error {
        return format!("AwayGuard — error: {e}");
    }
    let armed = if status.armed { "armed" } else { "not armed" };
    let presence = match status.presence {
        Presence::Unknown => "unknown",
        Presence::Near => "near",
        Presence::Away => "away",
    };
    format!("AwayGuard — {armed} · {presence}")
}

/// Applies `config`'s thresholds to `tracker`, but only when they actually
/// differ from `current` (the thresholds already in effect on `tracker`).
///
/// This is how the polling loop picks up a user's edited
/// near_dbm/away_dbm/confirm_samples (via `set_config`) without an app
/// restart. It is deliberately conditional: `ProximityTracker::set_thresholds`
/// preserves the EMA and the in-progress confirmation streak, so calling it
/// is harmless on its own, but comparing first keeps "only touch tracker
/// state on a real config change" true even if that guarantee ever changes,
/// and avoids a write to `tracker` on every single poll.
fn sync_thresholds(tracker: &mut ProximityTracker, current: &mut Thresholds, config: &Config) {
    let new = Thresholds {
        near_dbm: config.near_dbm,
        away_dbm: config.away_dbm,
        confirm_samples: config.confirm_samples,
    };
    if new.near_dbm != current.near_dbm
        || new.away_dbm != current.away_dbm
        || new.confirm_samples != current.confirm_samples
    {
        tracker.set_thresholds(new);
        *current = new;
    }
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

            // Built before AppState so the tray handle can be stored in it
            // and used by the polling loop to keep the tooltip live.
            let tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("AwayGuard — starting…")
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
                tray,
            };
            app.manage(state);

            // Polling loop.
            let handle = app.handle().clone();
            let poll_task = tauri::async_runtime::spawn(async move {
                let state = handle.state::<AppState>();
                let mut current_thresholds = {
                    let c = state.config.lock().unwrap();
                    Thresholds {
                        near_dbm: c.near_dbm,
                        away_dbm: c.away_dbm,
                        confirm_samples: c.confirm_samples,
                    }
                };
                let mut tracker = ProximityTracker::new(current_thresholds);
                let mut grace = GraceTimer::new();
                loop {
                    tokio::time::sleep(POLL_INTERVAL).await;
                    let (armed, target, grace_seconds) = {
                        let c = state.config.lock().unwrap();
                        // Pick up any near_dbm/away_dbm/confirm_samples edit made
                        // via set_config since the last round, without resetting
                        // the tracker's accumulated EMA/streak.
                        sync_thresholds(&mut tracker, &mut current_thresholds, &c);
                        (c.armed, c.target_id.clone(), c.grace_seconds)
                    };
                    let Some(target) = target else {
                        // No device selected yet: previously this bare
                        // `continue`d, leaving status/tray stuck on
                        // whatever they last showed forever -- including a
                        // ticked "armed" checkbox with nothing actually
                        // being monitored (IMPORTANT 1). Surface it.
                        let status = MonitorStatus {
                            presence: tracker.state(),
                            rssi: tracker.smoothed_rssi(),
                            armed,
                            error: Some("no device selected".to_string()),
                        };
                        *state.status.lock().unwrap() = status.clone();
                        let _ = state.tray.set_tooltip(Some(tray_tooltip(&status)));
                        let _ = handle.emit("status", status);
                        continue;
                    };
                    let status = run_once(
                        state.source.as_ref(),
                        &mut tracker,
                        &mut grace,
                        state.locker.as_ref(),
                        armed,
                        &target,
                        grace_seconds,
                        POLL_INTERVAL,
                    )
                    .await;
                    *state.status.lock().unwrap() = status.clone();
                    let _ = state.tray.set_tooltip(Some(tray_tooltip(&status)));
                    let _ = handle.emit("status", status);
                }
            });

            // LEDGER 2: the polling task's JoinHandle was previously
            // discarded, so a panic in it killed monitoring silently --
            // state.status kept its last value forever and the UI kept
            // serving a stale "everything is fine" snapshot. Await it from
            // a supervisor task instead: the loop above never returns
            // normally, so reaching here means it died, and that must be
            // surfaced exactly like any other broken sensing chain.
            let watchdog_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let result = poll_task.await;
                let state = watchdog_handle.state::<AppState>();
                let message = match result {
                    Ok(()) => "monitoring stopped unexpectedly".to_string(),
                    Err(e) => format!("monitoring stopped: {e}"),
                };
                let status = {
                    let mut s = state.status.lock().unwrap();
                    s.error = Some(message);
                    s.clone()
                };
                let _ = state.tray.set_tooltip(Some(tray_tooltip(&status)));
                let _ = watchdog_handle.emit("status", status);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ble::FakeSource;
    use crate::lock::LockBackend;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RecordingLocker {
        calls: AtomicUsize,
    }
    impl RecordingLocker {
        fn new() -> Self {
            Self { calls: AtomicUsize::new(0) }
        }
        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }
    impl ScreenLocker for RecordingLocker {
        fn lock(&self) -> Result<(), String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn backend(&self) -> LockBackend {
            LockBackend::PrivateApi
        }
    }

    fn config_with(near_dbm: i16, away_dbm: i16, confirm_samples: u8) -> Config {
        Config {
            target_id: Some("fake-device".into()),
            target_name: None,
            near_dbm,
            away_dbm,
            confirm_samples,
            grace_seconds: 10,
            armed: true,
        }
    }

    #[tokio::test]
    async fn threshold_change_takes_effect_without_restart() {
        // Mirrors the polling loop: one ProximityTracker/Thresholds pair
        // carried across many rounds, sync_thresholds called each round
        // exactly as run()'s spawned task does.
        let mut current = Thresholds { near_dbm: -70, away_dbm: -85, confirm_samples: 3 };
        let mut tracker = ProximityTracker::new(current);
        let locker = RecordingLocker::new();

        let mut grace = GraceTimer::new();

        // Establish Near under the original thresholds.
        let original = config_with(-70, -85, 3);
        let source = FakeSource::new(vec![Some(-40), Some(-40), Some(-40)]);
        for _ in 0..3 {
            sync_thresholds(&mut tracker, &mut current, &original);
            run_once(&source, &mut tracker, &mut grace, &locker, original.armed, "fake-device", original.grace_seconds, POLL_INTERVAL).await;
        }
        assert_eq!(tracker.state(), Presence::Near);

        // The user tightens near_dbm via set_config while the app keeps
        // running -- no restart. The same -40 dBm signal must now read
        // Away under the new thresholds.
        let tightened = config_with(-30, -35, 3);
        let source2 = FakeSource::new(vec![Some(-40), Some(-40), Some(-40)]);
        let mut saw_away = false;
        for _ in 0..3 {
            sync_thresholds(&mut tracker, &mut current, &tightened);
            let status =
                run_once(&source2, &mut tracker, &mut grace, &locker, tightened.armed, "fake-device", tightened.grace_seconds, POLL_INTERVAL).await;
            if status.presence == Presence::Away {
                saw_away = true;
            }
        }
        assert!(
            saw_away,
            "an edited threshold must take effect on the running tracker, not just after a restart"
        );
    }

    #[tokio::test]
    async fn steady_state_polling_does_not_reset_the_confirmation_streak() {
        // Guards against the trap of resetting the tracker's EMA/streak on
        // every loop iteration (e.g. rebuilding ProximityTracker::new(..)
        // unconditionally each round): if it did, this departure would
        // never confirm and the locker would never be called.
        let mut current = Thresholds { near_dbm: -70, away_dbm: -85, confirm_samples: 3 };
        let mut tracker = ProximityTracker::new(current);
        let mut grace = GraceTimer::new();
        let locker = RecordingLocker::new();
        let config = config_with(-70, -85, 3);

        // Establish Near.
        let establish = FakeSource::new(vec![Some(-40), Some(-40), Some(-40)]);
        for _ in 0..3 {
            sync_thresholds(&mut tracker, &mut current, &config);
            run_once(&establish, &mut tracker, &mut grace, &locker, config.armed, "fake-device", config.grace_seconds, POLL_INTERVAL).await;
        }
        assert_eq!(tracker.state(), Presence::Near);

        // Depart over many rounds with an UNCHANGED config each round --
        // exactly what a real 2-second polling loop does while the user
        // isn't touching the UI. config.grace_seconds is 10 (config_with)
        // and POLL_INTERVAL is 2s, so the confirmed departure (around round
        // 6) plus 4 more rounds (10s of grace) still fits comfortably
        // inside these 12 rounds.
        let depart = FakeSource::new(vec![Some(-100)]); // FakeSource repeats the last value
        let mut locked = false;
        for _ in 0..12 {
            sync_thresholds(&mut tracker, &mut current, &config);
            let status = run_once(&depart, &mut tracker, &mut grace, &locker, config.armed, "fake-device", config.grace_seconds, POLL_INTERVAL).await;
            if status.presence == Presence::Away {
                locked = true;
            }
        }
        assert!(
            locked,
            "departure must still confirm after the normal number of samples under steady-state polling"
        );
        assert_eq!(locker.calls(), 1, "must lock exactly once");
    }
}
