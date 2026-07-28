# AwayGuard Design

**Date:** 2026-07-28
**Status:** Approved

## Goal

A macOS menu bar app that locks the screen automatically when the user walks away, using Bluetooth proximity to their iPhone. No manual lock, no idle timer.

## Stack

- **Shell:** Tauri 2 (Rust backend, WKWebView frontend)
- **Frontend:** Svelte 5 + Vite + TypeScript
- **Sensing:** `btleplug` (BLE scanning)
- **Lock:** `SACLockScreenImmediate` via `dlopen`, falling back to `CGSession -suspend`

## Sensing: how proximity is actually measured

The Mac runs a BLE scan and tracks one peripheral — the user's iPhone — identified by its
**stable `CBPeripheral` UUID**, persisted in config. RSSI from that peripheral's advertisements
is the proximity signal.

This works because the iPhone is **bonded** to the Mac. macOS stores the phone's IRK and resolves
its rotating private address, so the scan surfaces the phone under its real name with a UUID that
is stable across app restarts. BLE MAC randomization, which would defeat this for an unbonded
phone, is not an obstacle here.

### Rejected: classic Bluetooth RSSI

The original design polled `IOBluetoothDevice::rawRSSI()` for the paired iPhone. It was
implemented as a probe and **disproved by a physical walk test**: with the phone carried out of
range for a minute, the value stayed frozen at exactly -39 across 90 consecutive samples. It is a
cached value, not a live measurement, and never reports departure.

Additional traps confirmed in that framework, recorded so they are not rediscovered: an absent
device reads `0` (not a weak negative); `isConnected()` is `false` even for a linked iPhone that
returns a valid RSSI; `openConnection()` against an iPhone fails with `kIOReturnTimeout`; and
`pairedDevices()` returns duplicate entries per address.

### Measured characteristics that shape the design

- RSSI is live and **noisy**: -33, -35, -55, -35, -46, -35 across consecutive samples while
  roughly stationary — a ~20 dB spread.
- The RF environment is crowded: **251 BLE devices** in a single scan. Filtering must be by stored
  UUID, never by name.

The noise is the central design constraint. A single threshold comparison would flap and fire
spurious locks. Departure detection therefore requires smoothing, dual-threshold hysteresis, and
consecutive-sample confirmation.

## Architecture

```
src/                     Svelte tray popover UI
src-tauri/src/
  ble.rs         BLE scanning via btleplug; discovery + RSSI sampling
  proximity.rs   Pure state machine: sample stream -> Near/Away/Unknown
  lock.rs        ScreenLocker trait + macOS implementation
  monitor.rs     Orchestration loop; ties sensing -> state -> lock -> events
  config.rs      Persisted settings (serde JSON)
  commands.rs    #[tauri::command] IPC surface
```

The layering exists to isolate the one piece worth testing properly. `proximity.rs` is pure — no
I/O, no system APIs, no clock of its own — so the departure logic is unit-testable without
hardware. Sensing and locking sit behind traits so `monitor.rs` can be exercised with fakes, and
so the sensing backend can be replaced without touching anything else.

## Proximity state machine

Three states: `Near`, `Away`, `Unknown`. Inputs are `Option<i16>` samples — `None` means the
peripheral was not seen in that scan round, which is distinct from a weak reading.

Transitions use dual thresholds that do not overlap:

- `Near -> Away` requires the smoothed RSSI below `away_threshold`, or `None`, for
  `confirm_samples` consecutive rounds.
- `Away -> Near` requires smoothed RSSI above `near_threshold` (higher than `away_threshold`).

The gap between thresholds is the hysteresis band that prevents oscillation at the boundary. After
`Away` is confirmed, a configurable grace period elapses before the lock fires, so briefly stepping
out of range does not lock the machine.

## Locking

`ScreenLocker` is a trait. The macOS implementation `dlopen`s
`/System/Library/PrivateFrameworks/login.framework/…/login` and calls `SACLockScreenImmediate`.
If the symbol cannot be resolved — a plausible outcome on a future macOS — it falls back to
executing `CGSession -suspend`. Which path is active is surfaced in the UI, so the user is never
misled about whether a real lock will happen.

## Failure posture

The security-relevant property: while armed, a broken monitoring chain must **fail visibly, not
silently**. Bluetooth off, adapter missing, target peripheral never seen, or lock symbol
unresolvable all surface as an explicit error state in the tray icon and popover. The app never
locks spuriously because data stopped arriving, and never pretends it is guarding when it is not.

## UI

A single tray popover: device picker, live RSSI readout, threshold sliders, arm/disarm toggle, and
current status. The frontend calls commands via `invoke` and receives state changes via `listen`.
`ActivationPolicy::Accessory` keeps the app out of the Dock.

## Testing

- `proximity.rs`: real unit tests covering hysteresis, confirmation counting, `None` handling, and
  grace-period behavior. This is where correctness lives.
- `monitor.rs`: driven by fake sensing and a recording locker to assert that lock fires exactly
  once per departure.
- `ble.rs` and `lock.rs`: thin FFI/IO wrappers, verified manually against real hardware.
