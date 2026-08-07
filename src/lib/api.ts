import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type Presence = "unknown" | "near" | "away";
export type LockBackend = "privateApi" | "screenSaver" | "unavailable";
export type SettingsPane = "bluetooth" | "lockScreen";

export interface DiscoveredDevice { id: string; name: string; rssi: number | null }
export interface Config {
  target_id: string | null;
  target_name: string | null;
  near_dbm: number;
  away_dbm: number;
  confirm_samples: number;
  grace_seconds: number;
  armed: boolean;
}
export interface MonitorStatus {
  presence: Presence;
  rssi: number | null;
  armed: boolean;
  error: string | null;
  /** Whole seconds until the pending lock fires; null when nothing is counting down. */
  grace_remaining: number | null;
  /** Seconds since the target last advertised; null until it has been seen at all. */
  last_seen: number | null;
  /** The state the tracker is accumulating evidence for, and how many samples of it. */
  pending: Presence | null;
  pending_samples: number;
  /** Poll cadence, so the UI can say how fresh these numbers are. */
  poll_interval: number;
}

export const listDevices = () => invoke<DiscoveredDevice[]>("list_devices");
export const getConfig = () => invoke<Config>("get_config");
export const setConfig = (config: Config) => invoke<void>("set_config", { config });
export const getStatus = () => invoke<MonitorStatus>("get_status");
export const lockBackend = () => invoke<LockBackend>("lock_backend");
/** "Cancel — I'm still here". Dismisses one pending lock; does not disarm. */
export const cancelPendingLock = () => invoke<void>("cancel_pending_lock");
export const openSettings = (pane: SettingsPane) => invoke<void>("open_settings", { pane });
export const quit = () => invoke<void>("quit");
export const onStatus = (cb: (s: MonitorStatus) => void) =>
  listen<MonitorStatus>("status", (e) => cb(e.payload));
