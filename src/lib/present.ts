/*
 * Turns backend state into the words and numbers the popover shows.
 *
 * Kept out of the component so the design's central rule is inspectable in
 * one place: the headline reports what the backend is actually doing, never
 * what the user asked for. If the arm toggle is on but Bluetooth is off, the
 * headline still reads "Not protected" — the switch is a wish, the headline
 * is a fact.
 */

import type { Config, LockBackend, MonitorStatus, Presence, SettingsPane } from "$lib/api";

/** Smallest threshold band the UI will let the user create, in dB.
 *
 * Must match the server-side floor in src-tauri/src/config.rs
 * (`Config::normalize_thresholds`'s `MIN_THRESHOLD_GAP_DBM`). Enforcing it
 * here is a UX nicety only — the backend re-enforces it on every set_config
 * regardless, since it is the one that must never fail open. Measured RSSI
 * spread on real hardware is ~20 dB, so this has to be a genuinely usable
 * band and not merely a non-empty one. */
export const MIN_GAP_DBM = 12;

/** The dBm domain both threshold handles and the signal meter are drawn on,
 * per the design's component inventory ("−100 → −30 dBm"). */
export const RSSI_FLOOR = -100;
export const RSSI_CEIL = -30;

export type Tone = "neutral" | "near" | "away" | "warn" | "danger";
export type ShieldState = "outline" | "solid" | "open" | "slashed";

export interface Headline {
  tone: Tone;
  title: string;
  detail: string;
  /** Whether to show the state dot beside the title. Only protected states
   * earn one; "Not protected" gets no reassuring ornament. */
  dot: boolean;
}

export interface BannerSpec {
  tone: "danger" | "warn";
  title: string;
  body: string;
  action: { label: string; pane: SettingsPane } | null;
}

/** Position of a dBm reading along the shared −100…−30 track, as a percent. */
export function pct(dbm: number): number {
  const t = (dbm - RSSI_FLOOR) / (RSSI_CEIL - RSSI_FLOOR);
  return Math.max(0, Math.min(1, t)) * 100;
}

export function deviceLabel(config: Config | null): string {
  return config?.target_name ?? "your phone";
}

/** "3s" / "4 minutes" / "2 hours" — coarsens as it ages, because "seen 4,120
 * seconds ago" is a number nobody reads. */
export function ago(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} minute${minutes === 1 ? "" : "s"}`;
  const hours = Math.round(minutes / 60);
  return `${hours} hour${hours === 1 ? "" : "s"}`;
}

function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? "" : "s"}`;
}

/** True when the backend's error is just "nothing to monitor yet". That is a
 * setup step, not a fault, and the empty device row already says it. */
export function isNoDeviceError(error: string | null): boolean {
  return error?.toLowerCase().includes("no device selected") ?? false;
}

export function headline(
  status: MonitorStatus | null,
  config: Config | null,
  backend: LockBackend | null,
): Headline {
  if (!config?.target_id) {
    return {
      tone: "neutral",
      title: "Not protected",
      detail: "AwayGuard isn’t watching for anything yet.",
      dot: false,
    };
  }
  if (backend === "unavailable") {
    return {
      tone: "danger",
      title: "Not protected",
      detail: "This Mac has no lock mechanism AwayGuard can use.",
      dot: false,
    };
  }
  if (status?.error && !isNoDeviceError(status.error)) {
    return {
      tone: "danger",
      title: "Not protected",
      detail: "Sensing is broken — treat this Mac as unguarded.",
      dot: false,
    };
  }
  if (!config.armed) {
    return {
      tone: "neutral",
      title: "Not protected",
      detail: `${deviceLabel(config)} is selected, but the guard is switched off.`,
      dot: false,
    };
  }
  if (status?.grace_remaining != null) {
    const gone = Math.max(0, config.grace_seconds - status.grace_remaining);
    return {
      tone: "away",
      title: `Locking in ${status.grace_remaining}s`,
      detail: `${deviceLabel(config)} has been out of range for ${plural(gone, "second")}.`,
      dot: false,
    };
  }
  if (backend === "screenSaver") {
    return {
      tone: "warn",
      title: "Protected, with limits",
      detail: "Screen saver fallback — a password may not be required.",
      dot: true,
    };
  }
  return {
    tone: "near",
    title: "Protected",
    detail: `Your Mac locks ${plural(config.grace_seconds, "second")} after ${deviceLabel(
      config,
    )} leaves range.`,
    dot: true,
  };
}

/** The banner above the headline, if the truth needs one. At most one shows:
 * a broken sensor outranks a degraded lock, because it means nothing is
 * being watched at all. */
export function banner(
  status: MonitorStatus | null,
  config: Config | null,
  backend: LockBackend | null,
): BannerSpec | null {
  if (backend === "unavailable") {
    return {
      tone: "danger",
      title: "No lock mechanism available",
      body: "AwayGuard has no way to lock this Mac, so arming it would do nothing.",
      action: null,
    };
  }
  const error = status?.error;
  if (error && !isNoDeviceError(error)) {
    // The adapter's own words vary by failure; recognising the Bluetooth
    // ones is what lets us offer the button that actually fixes it.
    const bluetooth = /bluetooth|powered off|poweredoff|adapter|unauthori|permission|denied/i.test(
      error,
    );
    return {
      tone: "danger",
      title: bluetooth ? "Bluetooth is off — not monitoring" : "Not monitoring",
      // The adapter's own words, and nothing else: the headline right below
      // already says what it means for the user, and the banner has to stay
      // short enough that the controls under it survive at 420pt.
      body: error,
      action: bluetooth ? { label: "Open Bluetooth settings", pane: "bluetooth" } : null,
    };
  }
  if (backend === "screenSaver") {
    return {
      tone: "warn",
      title: "Lock isn’t guaranteed",
      body: "Turn on “Require password after screen saver begins,” or the Mac stays open.",
      action: { label: "Open Lock Screen settings", pane: "lockScreen" },
    };
  }
  return null;
}

/** Menu bar glyph. Shape carries the state — filled, open, slashed — so it
 * survives monochrome rendering and colour blindness. */
export function shieldState(
  status: MonitorStatus | null,
  config: Config | null,
  backend: LockBackend | null,
): ShieldState {
  if (backend === "unavailable") return "slashed";
  if (status?.error && !isNoDeviceError(status.error)) return "slashed";
  if (!config?.target_id || !config.armed) return "outline";
  if (status?.grace_remaining != null || status?.presence === "away") return "open";
  return "solid";
}

export function presenceLabel(presence: Presence): string {
  return { unknown: "Unknown", near: "Near", away: "Away" }[presence];
}

/** One word for the current reading, judged against the user's own
 * thresholds rather than an absolute scale — "weak" means weak enough to
 * count as away *here*. */
export function signalWord(rssi: number, config: Config): "strong" | "steady" | "weak" {
  if (rssi >= config.near_dbm) return "strong";
  if (rssi <= config.away_dbm) return "weak";
  return "steady";
}

/** The line under the signal meter. Says the most specific true thing
 * available: a transition in progress beats a freshness timestamp, because
 * it is the thing that is about to change the user's protection. */
export function signalCaption(status: MonitorStatus | null): string {
  if (!status) return "Waiting for the first sample";
  if (status.pending && status.pending_samples > 0) {
    const edge = status.pending === "away" ? "Below the away" : "Above the near";
    return `${edge} threshold for ${plural(status.pending_samples, "sample")}`;
  }
  if (status.last_seen == null) return "Never seen";
  const seen = status.last_seen === 0 ? "Seen just now" : `Seen ${ago(status.last_seen)} ago`;
  return `${seen} · sampling every ${status.poll_interval}s`;
}

/** The same line, shortened to sit beside the meter on one row when a banner
 * has taken the vertical space. Drops the poll cadence — the least urgent
 * part — and keeps freshness, which is what a broken sensor calls into
 * question in the first place. */
export function signalCaptionShort(status: MonitorStatus | null): string {
  if (!status) return "waiting";
  if (status.pending && status.pending_samples > 0) {
    const edge = status.pending === "away" ? "below away" : "above near";
    return `${plural(status.pending_samples, "sample")} ${edge}`;
  }
  if (status.last_seen == null) return "never seen";
  return status.last_seen === 0 ? "seen just now" : `seen ${ago(status.last_seen)} ago`;
}

/** Clamp a threshold pair back to the minimum band by *pushing* the other
 * handle rather than refusing the drag — the band never collapses, and the
 * handle you are holding always follows your pointer.
 *
 * `moved` names the handle the user is dragging, so the other one gives way.
 */
export function enforceGap(
  away: number,
  near: number,
  moved: "away" | "near",
): { away: number; near: number } {
  if (near - away >= MIN_GAP_DBM) return { away, near };
  if (moved === "away") {
    const pushed = Math.min(RSSI_CEIL, away + MIN_GAP_DBM);
    return { away: Math.min(away, pushed - MIN_GAP_DBM), near: pushed };
  }
  const pushed = Math.max(RSSI_FLOOR, near - MIN_GAP_DBM);
  return { away: pushed, near: Math.max(near, pushed + MIN_GAP_DBM) };
}
