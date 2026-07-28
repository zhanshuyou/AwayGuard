import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type Presence = "unknown" | "near" | "away";
export type LockBackend = "privateApi" | "screenSaver" | "unavailable";

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
}

export const listDevices = () => invoke<DiscoveredDevice[]>("list_devices");
export const getConfig = () => invoke<Config>("get_config");
export const setConfig = (config: Config) => invoke<void>("set_config", { config });
export const getStatus = () => invoke<MonitorStatus>("get_status");
export const lockBackend = () => invoke<LockBackend>("lock_backend");
export const onStatus = (cb: (s: MonitorStatus) => void) =>
  listen<MonitorStatus>("status", (e) => cb(e.payload));
