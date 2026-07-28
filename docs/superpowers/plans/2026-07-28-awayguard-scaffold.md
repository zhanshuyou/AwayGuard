# AwayGuard Scaffold Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build AwayGuard as a macOS menu bar app that locks the screen when the user's iPhone goes out of Bluetooth range, working end to end.

**Architecture:** A Tauri 2 app with a Rust backend and a SvelteKit tray popover. BLE scanning (`btleplug`) samples the RSSI of one bonded iPhone, identified by its stable `CBPeripheral` UUID. A pure state machine converts that noisy sample stream into Near/Away with smoothing and hysteresis, and a departure fires a screen lock through a `dlopen`'d private macOS symbol.

**Tech Stack:** Rust (Tauri 2, btleplug, tokio, serde), SvelteKit 2 + Svelte 5 + TypeScript, Vite 6, pnpm.

## Global Constraints

- Target platform is **macOS only**. Do not add Windows/Linux branches.
- **Xcode is NOT installed** — only Command Line Tools. `xcodebuild` is unavailable. Everything must build via `cargo` / `pnpm tauri`.
- Package manager is **pnpm**. Do not use npm or yarn for project commands.
- Bundle identifier is exactly `com.shuyou.awayguard`.
- Rust edition **2021** (as generated). Rust toolchain is 1.95.0.
- The repository already contains `README.md`, `LICENSE`, `CLAUDE.md`, and `docs/`. **Never overwrite these** when scaffolding.
- The sensing layer must sit behind the `ProximitySource` trait. No other module may call `btleplug` directly.
- Never identify the target phone by name — always by stored peripheral UUID. A single scan sees ~250 devices.
- All measured platform facts in this plan were verified on macOS 26.0 on 2026-07-28. Do not "correct" them from memory.

---

### Task 1: Scaffold the Tauri + SvelteKit project

**Files:**
- Create: `package.json`, `svelte.config.js`, `vite.config.js`, `tsconfig.json`, `.gitignore`
- Create: `src/app.html`, `src/routes/+page.svelte`, `src/routes/+layout.ts`
- Create: `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`, `src-tauri/build.rs`
- Create: `src-tauri/src/main.rs`, `src-tauri/src/lib.rs`, `src-tauri/capabilities/default.json`

**Interfaces:**
- Consumes: nothing (first task)
- Produces: a buildable crate named `awayguard` with lib name `awayguard_lib`, and `pnpm tauri dev` as the run command. All later tasks add modules under `src-tauri/src/`.

- [ ] **Step 1: Generate the project in a temp directory**

The generator refuses to merge cleanly into a non-empty repo, and its own `README.md` would clobber ours. Generate outside the repo first.

```bash
TMP=$(mktemp -d)
cd "$TMP"
npm create tauri-app@latest awayguard -- \
  --template svelte-ts \
  --manager pnpm \
  --identifier com.shuyou.awayguard \
  --tauri-version 2 \
  --yes
```

- [ ] **Step 2: Copy into the repo, preserving existing files**

```bash
rsync -a --exclude README.md "$TMP/awayguard/" /Users/zilliz/Project/AwayGuard/
cd /Users/zilliz/Project/AwayGuard
git status --short
```

Expected: new untracked files (`package.json`, `src/`, `src-tauri/`, …). `README.md`, `LICENSE`, `CLAUDE.md`, and `docs/` must appear **unmodified**.

- [ ] **Step 3: Add Rust build artifacts to .gitignore**

The generated `.gitignore` covers the frontend but not Cargo's output. Append:

```gitignore
/src-tauri/target
/src-tauri/gen/schemas
```

- [ ] **Step 4: Install dependencies and verify the Rust side compiles**

```bash
pnpm install
cd src-tauri && cargo check 2>&1 | tail -20
```

Expected: `Finished` with no errors. This is the moment that proves Tauri builds without Xcode — if it fails on a missing `xcodebuild`, stop and report rather than working around it.

- [ ] **Step 5: Verify the app actually launches**

```bash
pnpm tauri dev
```

Expected: a window opens showing the default Tauri + Svelte page. Close it. If the dev server hangs waiting on port 1420, confirm nothing else holds that port.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: scaffold Tauri 2 + SvelteKit project"
```

---

### Task 2: Proximity state machine

This is the only module with genuinely intricate logic, and it is pure — no I/O, no clock, no system APIs — so it is fully unit-testable. Measured RSSI swings ~20 dB while stationary (-33, -35, -55, -35, -46, -35), so naive thresholding **will** produce spurious locks. That is what these tests exist to prevent.

**Files:**
- Create: `src-tauri/src/proximity.rs`
- Modify: `src-tauri/src/lib.rs` (add `pub mod proximity;`)

**Interfaces:**
- Consumes: nothing
- Produces:
  - `pub enum Presence { Unknown, Near, Away }` (derives `Debug, Clone, Copy, PartialEq, Eq, serde::Serialize`)
  - `pub struct Thresholds { pub near_dbm: i16, pub away_dbm: i16, pub confirm_samples: u8 }`
  - `pub struct ProximityTracker`
  - `ProximityTracker::new(thresholds: Thresholds) -> Self`
  - `ProximityTracker::push(&mut self, sample: Option<i16>) -> Option<Presence>` — returns `Some(new_state)` only on a transition
  - `ProximityTracker::state(&self) -> Presence`
  - `ProximityTracker::smoothed_rssi(&self) -> Option<f32>`

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/proximity.rs` containing only the test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn thresholds() -> Thresholds {
        Thresholds { near_dbm: -70, away_dbm: -85, confirm_samples: 3 }
    }

    #[test]
    fn starts_unknown() {
        let t = ProximityTracker::new(thresholds());
        assert_eq!(t.state(), Presence::Unknown);
    }

    #[test]
    fn confirms_near_after_enough_strong_samples() {
        let mut t = ProximityTracker::new(thresholds());
        assert_eq!(t.push(Some(-40)), None);
        assert_eq!(t.push(Some(-40)), None);
        assert_eq!(t.push(Some(-40)), Some(Presence::Near));
        assert_eq!(t.state(), Presence::Near);
    }

    #[test]
    fn single_weak_sample_does_not_flip_to_away() {
        // The core anti-flap guarantee: measured RSSI dips ~20 dB spontaneously.
        let mut t = ProximityTracker::new(thresholds());
        for _ in 0..3 { t.push(Some(-40)); }
        assert_eq!(t.state(), Presence::Near);
        assert_eq!(t.push(Some(-95)), None);
        assert_eq!(t.state(), Presence::Near);
    }

    #[test]
    fn confirms_away_after_sustained_weak_samples() {
        let mut t = ProximityTracker::new(thresholds());
        for _ in 0..3 { t.push(Some(-40)); }
        let mut transition = None;
        for _ in 0..12 {
            if let Some(s) = t.push(Some(-100)) { transition = Some(s); break; }
        }
        assert_eq!(transition, Some(Presence::Away));
    }

    #[test]
    fn missing_sample_counts_as_away_evidence() {
        // None means the peripheral was not seen in that scan round,
        // which is different from a weak reading.
        let mut t = ProximityTracker::new(thresholds());
        for _ in 0..3 { t.push(Some(-40)); }
        let mut transition = None;
        for _ in 0..12 {
            if let Some(s) = t.push(None) { transition = Some(s); break; }
        }
        assert_eq!(transition, Some(Presence::Away));
    }

    #[test]
    fn samples_inside_hysteresis_band_hold_current_state() {
        let mut t = ProximityTracker::new(thresholds());
        for _ in 0..3 { t.push(Some(-40)); }
        for _ in 0..10 {
            assert_eq!(t.push(Some(-78)), None); // between away_dbm and near_dbm
        }
        assert_eq!(t.state(), Presence::Near);
    }

    #[test]
    fn returns_transition_only_once_per_change() {
        let mut t = ProximityTracker::new(thresholds());
        t.push(Some(-40));
        t.push(Some(-40));
        assert_eq!(t.push(Some(-40)), Some(Presence::Near));
        assert_eq!(t.push(Some(-40)), None);
        assert_eq!(t.push(Some(-40)), None);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd src-tauri && cargo test proximity 2>&1 | tail -20
```

Expected: compile errors — `cannot find type Thresholds`, `ProximityTracker not found`.

- [ ] **Step 3: Write the implementation**

Prepend to `src-tauri/src/proximity.rs`, above the test module:

```rust
use serde::Serialize;

/// Exponential smoothing factor. Low enough to ride out the ~20 dB
/// spontaneous dips measured on real hardware.
const SMOOTHING_ALPHA: f32 = 0.35;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Presence {
    Unknown,
    Near,
    Away,
}

#[derive(Debug, Clone, Copy)]
pub struct Thresholds {
    /// Smoothed RSSI at or above this counts as evidence of Near.
    pub near_dbm: i16,
    /// Smoothed RSSI at or below this counts as evidence of Away.
    pub away_dbm: i16,
    /// Consecutive rounds of consistent evidence required to change state.
    pub confirm_samples: u8,
}

pub struct ProximityTracker {
    thresholds: Thresholds,
    smoothed: Option<f32>,
    state: Presence,
    pending: Option<Presence>,
    streak: u8,
}

impl ProximityTracker {
    pub fn new(thresholds: Thresholds) -> Self {
        Self { thresholds, smoothed: None, state: Presence::Unknown, pending: None, streak: 0 }
    }

    pub fn state(&self) -> Presence {
        self.state
    }

    pub fn smoothed_rssi(&self) -> Option<f32> {
        self.smoothed
    }

    /// Feed one scan round. `None` means the peripheral was not seen at all.
    /// Returns `Some(new_state)` only on an actual transition.
    pub fn push(&mut self, sample: Option<i16>) -> Option<Presence> {
        let evidence = match sample {
            // Not seen at all is strong evidence of departure, and must not
            // pollute the smoothed value with a fabricated reading.
            None => Some(Presence::Away),
            Some(raw) => {
                let next = match self.smoothed {
                    Some(prev) => prev + SMOOTHING_ALPHA * (raw as f32 - prev),
                    None => raw as f32,
                };
                self.smoothed = Some(next);

                if next >= self.thresholds.near_dbm as f32 {
                    Some(Presence::Near)
                } else if next <= self.thresholds.away_dbm as f32 {
                    Some(Presence::Away)
                } else {
                    // Inside the hysteresis band: no evidence either way.
                    None
                }
            }
        };

        match evidence {
            Some(e) if e != self.state => {
                if self.pending == Some(e) {
                    self.streak = self.streak.saturating_add(1);
                } else {
                    self.pending = Some(e);
                    self.streak = 1;
                }
                if self.streak >= self.thresholds.confirm_samples {
                    self.state = e;
                    self.pending = None;
                    self.streak = 0;
                    return Some(e);
                }
                None
            }
            _ => {
                // Evidence agrees with current state, or is inconclusive.
                self.pending = None;
                self.streak = 0;
                None
            }
        }
    }
}
```

Add to `src-tauri/src/lib.rs`, above the existing `run()`:

```rust
pub mod proximity;
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd src-tauri && cargo test proximity 2>&1 | tail -20
```

Expected: `test result: ok. 7 passed`.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/proximity.rs src-tauri/src/lib.rs
git commit -m "feat: add proximity state machine with hysteresis"
```

---

### Task 3: Screen locker

**Files:**
- Create: `src-tauri/src/lock.rs`
- Modify: `src-tauri/src/lib.rs` (add `pub mod lock;`)
- Modify: `src-tauri/Cargo.toml` (add `libc`)

**Interfaces:**
- Consumes: nothing
- Produces:
  - `pub enum LockBackend { PrivateApi, ScreenSaver, Unavailable }` (derives `Debug, Clone, Copy, PartialEq, Eq, serde::Serialize`)
  - `pub trait ScreenLocker: Send + Sync { fn lock(&self) -> Result<(), String>; fn backend(&self) -> LockBackend; }`
  - `pub struct MacScreenLocker`
  - `MacScreenLocker::detect() -> Self`

**Verified platform facts — do not substitute remembered alternatives:**
- `SACLockScreenImmediate` **resolves** in `/System/Library/PrivateFrameworks/login.framework/Versions/A/login` on macOS 26.
- `CGSession` **does not exist**; `/System/Library/CoreServices/Menu Extras/User.menu/` was removed. Do not use it.
- `/System/Library/CoreServices/ScreenSaverEngine.app` exists and is the fallback.

- [ ] **Step 1: Add the libc dependency**

```bash
cd src-tauri && cargo add libc
```

- [ ] **Step 2: Write the failing test**

Create `src-tauri/src/lock.rs` with only the tests. These deliberately never call `lock()` — that would lock the developer's screen mid-test.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_a_usable_backend_on_this_machine() {
        let locker = MacScreenLocker::detect();
        assert_ne!(locker.backend(), LockBackend::Unavailable);
    }

    #[test]
    fn prefers_the_private_api_when_the_symbol_resolves() {
        // Verified present on macOS 26. If this ever fails, the fallback
        // path became load-bearing and the UI warning matters.
        assert_eq!(MacScreenLocker::detect().backend(), LockBackend::PrivateApi);
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

```bash
cd src-tauri && cargo test lock 2>&1 | tail -20
```

Expected: compile error — `cannot find MacScreenLocker`.

- [ ] **Step 4: Write the implementation**

Prepend to `src-tauri/src/lock.rs`:

```rust
use serde::Serialize;
use std::path::Path;
use std::process::Command;

const LOGIN_FRAMEWORK: &[u8] =
    b"/System/Library/PrivateFrameworks/login.framework/Versions/A/login\0";
const LOCK_SYMBOL: &[u8] = b"SACLockScreenImmediate\0";
const SCREENSAVER_APP: &str = "/System/Library/CoreServices/ScreenSaverEngine.app";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LockBackend {
    /// Immediate, real lock. Preferred.
    PrivateApi,
    /// Starts the screen saver. Only locks if the user enabled
    /// "require password after screen saver begins" — which we cannot verify,
    /// because com.apple.screensaver askForPassword is no longer readable.
    ScreenSaver,
    Unavailable,
}

pub trait ScreenLocker: Send + Sync {
    fn lock(&self) -> Result<(), String>;
    fn backend(&self) -> LockBackend;
}

type LockFn = unsafe extern "C" fn() -> i32;

fn resolve_lock_symbol() -> Option<LockFn> {
    unsafe {
        let handle = libc::dlopen(LOGIN_FRAMEWORK.as_ptr() as *const _, libc::RTLD_LAZY);
        if handle.is_null() {
            return None;
        }
        let sym = libc::dlsym(handle, LOCK_SYMBOL.as_ptr() as *const _);
        if sym.is_null() {
            None
        } else {
            Some(std::mem::transmute::<*mut libc::c_void, LockFn>(sym))
        }
    }
}

pub struct MacScreenLocker {
    backend: LockBackend,
}

impl MacScreenLocker {
    pub fn detect() -> Self {
        let backend = if resolve_lock_symbol().is_some() {
            LockBackend::PrivateApi
        } else if Path::new(SCREENSAVER_APP).exists() {
            LockBackend::ScreenSaver
        } else {
            LockBackend::Unavailable
        };
        Self { backend }
    }
}

impl ScreenLocker for MacScreenLocker {
    fn backend(&self) -> LockBackend {
        self.backend
    }

    fn lock(&self) -> Result<(), String> {
        match self.backend {
            LockBackend::PrivateApi => {
                let f = resolve_lock_symbol().ok_or("lock symbol disappeared at runtime")?;
                let rc = unsafe { f() };
                if rc == 0 {
                    Ok(())
                } else {
                    Err(format!("SACLockScreenImmediate returned {rc}"))
                }
            }
            LockBackend::ScreenSaver => Command::new("/usr/bin/open")
                .arg("-a")
                .arg(SCREENSAVER_APP)
                .status()
                .map_err(|e| format!("failed to start screen saver: {e}"))
                .and_then(|s| if s.success() { Ok(()) } else { Err("screen saver failed".into()) }),
            LockBackend::Unavailable => Err("no screen lock mechanism available".into()),
        }
    }
}
```

Add to `src-tauri/src/lib.rs`:

```rust
pub mod lock;
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cd src-tauri && cargo test lock 2>&1 | tail -20
```

Expected: `test result: ok. 2 passed`. Your screen must NOT lock during this run.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/lock.rs src-tauri/src/lib.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat: add screen locker with private API and screensaver fallback"
```

---

### Task 4: BLE proximity source

**Files:**
- Create: `src-tauri/src/ble.rs`
- Modify: `src-tauri/src/lib.rs` (add `pub mod ble;`)
- Modify: `src-tauri/Cargo.toml` (add `btleplug`, `tokio`, `async-trait`)

**Interfaces:**
- Consumes: nothing
- Produces:
  - `pub struct DiscoveredDevice { pub id: String, pub name: String, pub rssi: Option<i16> }` (derives `Debug, Clone, serde::Serialize`)
  - `#[async_trait] pub trait ProximitySource: Send + Sync { async fn discover(&self, secs: u64) -> Result<Vec<DiscoveredDevice>, String>; async fn sample(&self, target_id: &str) -> Result<Option<i16>, String>; }`
  - `pub struct BleSource`
  - `BleSource::new() -> Result<Self, String>` — **async**; call sites must `.await` it (Task 7 uses `tauri::async_runtime::block_on`)
  - `pub struct FakeSource` (test double used by Task 6): `FakeSource::new(samples: Vec<Option<i16>>) -> Self`

**Verified behavior this must match:** a bonded iPhone appears in a plain scan under its real name with a peripheral UUID stable across processes, and live RSSI. A scan sees ~250 devices, so `discover` must filter to named devices and `sample` must match on `id` only.

- [ ] **Step 1: Add dependencies**

```bash
cd src-tauri
cargo add btleplug async-trait
cargo add tokio --features rt-multi-thread,macros,time,sync
```

- [ ] **Step 2: Write the failing test**

Create `src-tauri/src/ble.rs` with only the tests. These exercise `FakeSource`, not real hardware — the real scan is verified manually in Step 5.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_source_replays_samples_in_order() {
        let src = FakeSource::new(vec![Some(-40), Some(-90), None]);
        assert_eq!(src.sample("any").await.unwrap(), Some(-40));
        assert_eq!(src.sample("any").await.unwrap(), Some(-90));
        assert_eq!(src.sample("any").await.unwrap(), None);
    }

    #[tokio::test]
    async fn fake_source_repeats_last_sample_when_exhausted() {
        let src = FakeSource::new(vec![Some(-50)]);
        assert_eq!(src.sample("any").await.unwrap(), Some(-50));
        assert_eq!(src.sample("any").await.unwrap(), Some(-50));
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

```bash
cd src-tauri && cargo test ble 2>&1 | tail -20
```

Expected: compile error — `cannot find FakeSource`.

- [ ] **Step 4: Write the implementation**

Prepend to `src-tauri/src/ble.rs`:

```rust
use async_trait::async_trait;
use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Adapter, Manager};
use serde::Serialize;
use std::sync::Mutex;
use std::time::Duration;

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredDevice {
    pub id: String,
    pub name: String,
    pub rssi: Option<i16>,
}

#[async_trait]
pub trait ProximitySource: Send + Sync {
    /// Scan for `secs` and return named devices, strongest signal first.
    async fn discover(&self, secs: u64) -> Result<Vec<DiscoveredDevice>, String>;
    /// Current RSSI for one peripheral. `Ok(None)` means "not seen this round".
    async fn sample(&self, target_id: &str) -> Result<Option<i16>, String>;
}

pub struct BleSource {
    central: Adapter,
}

impl BleSource {
    pub async fn new() -> Result<Self, String> {
        let manager = Manager::new().await.map_err(|e| e.to_string())?;
        let central = manager
            .adapters()
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .next()
            .ok_or("no Bluetooth adapter found")?;
        central
            .start_scan(ScanFilter::default())
            .await
            .map_err(|e| e.to_string())?;
        Ok(Self { central })
    }
}

#[async_trait]
impl ProximitySource for BleSource {
    async fn discover(&self, secs: u64) -> Result<Vec<DiscoveredDevice>, String> {
        tokio::time::sleep(Duration::from_secs(secs)).await;
        let mut out = Vec::new();
        for p in self.central.peripherals().await.map_err(|e| e.to_string())? {
            if let Ok(Some(props)) = p.properties().await {
                // ~250 devices show up per scan; only named ones are pickable.
                if let Some(name) = props.local_name {
                    out.push(DiscoveredDevice { id: p.id().to_string(), name, rssi: props.rssi });
                }
            }
        }
        out.sort_by_key(|d| -d.rssi.unwrap_or(-127));
        Ok(out)
    }

    async fn sample(&self, target_id: &str) -> Result<Option<i16>, String> {
        for p in self.central.peripherals().await.map_err(|e| e.to_string())? {
            if p.id().to_string() == target_id {
                if let Ok(Some(props)) = p.properties().await {
                    return Ok(props.rssi);
                }
            }
        }
        Ok(None)
    }
}

/// Test double: replays a fixed sample sequence, repeating the last value.
pub struct FakeSource {
    samples: Mutex<std::collections::VecDeque<Option<i16>>>,
    last: Mutex<Option<i16>>,
}

impl FakeSource {
    pub fn new(samples: Vec<Option<i16>>) -> Self {
        Self { samples: Mutex::new(samples.into()), last: Mutex::new(None) }
    }
}

#[async_trait]
impl ProximitySource for FakeSource {
    async fn discover(&self, _secs: u64) -> Result<Vec<DiscoveredDevice>, String> {
        Ok(vec![DiscoveredDevice {
            id: "fake-device".into(),
            name: "Fake iPhone".into(),
            rssi: Some(-40),
        }])
    }

    async fn sample(&self, _target_id: &str) -> Result<Option<i16>, String> {
        let mut q = self.samples.lock().unwrap();
        match q.pop_front() {
            Some(v) => {
                *self.last.lock().unwrap() = v;
                Ok(v)
            }
            None => Ok(*self.last.lock().unwrap()),
        }
    }
}
```

Add to `src-tauri/src/lib.rs`:

```rust
pub mod ble;
```

- [ ] **Step 5: Run tests, then verify against real hardware**

```bash
cd src-tauri && cargo test ble 2>&1 | tail -20
```

Expected: `test result: ok. 2 passed`.

Then confirm the real scan still sees the phone — the whole design depends on it:

```bash
cd src-tauri && cargo test --lib -- --ignored --nocapture 2>&1 | tail -5 || true
```

If no manual check is wired up, defer hardware verification to Task 7, where the UI device picker exercises `discover()` directly.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/ble.rs src-tauri/src/lib.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat: add BLE proximity source behind a trait"
```

---

### Task 5: Persisted configuration

**Files:**
- Create: `src-tauri/src/config.rs`
- Modify: `src-tauri/src/lib.rs` (add `pub mod config;`)

**Interfaces:**
- Consumes: nothing
- Produces:
  - `pub struct Config { pub target_id: Option<String>, pub target_name: Option<String>, pub near_dbm: i16, pub away_dbm: i16, pub confirm_samples: u8, pub grace_seconds: u64, pub armed: bool }` (derives `Debug, Clone, serde::Serialize, serde::Deserialize`)
  - `impl Default for Config`
  - `Config::load(dir: &std::path::Path) -> Config`
  - `Config::save(&self, dir: &std::path::Path) -> Result<(), String>`

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/config.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_disarmed_with_no_target() {
        let c = Config::default();
        assert!(!c.armed);
        assert_eq!(c.target_id, None);
        // Hysteresis band must be non-empty or the state machine flaps.
        assert!(c.near_dbm > c.away_dbm);
    }

    #[test]
    fn load_returns_defaults_when_file_is_missing() {
        let dir = std::env::temp_dir().join("awayguard-test-missing");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(Config::load(&dir).armed, false);
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = std::env::temp_dir().join("awayguard-test-roundtrip");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut c = Config::default();
        c.target_id = Some("18b1c44c-6313-7e59-ccfb-4aa35e765c2d".into());
        c.armed = true;
        c.grace_seconds = 12;
        c.save(&dir).unwrap();

        let back = Config::load(&dir);
        assert_eq!(back.target_id.as_deref(), Some("18b1c44c-6313-7e59-ccfb-4aa35e765c2d"));
        assert!(back.armed);
        assert_eq!(back.grace_seconds, 12);
    }

    #[test]
    fn load_falls_back_to_defaults_on_corrupt_file() {
        let dir = std::env::temp_dir().join("awayguard-test-corrupt");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.json"), b"{ not json").unwrap();
        // Must not panic, and must not silently arm.
        assert!(!Config::load(&dir).armed);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd src-tauri && cargo test config 2>&1 | tail -20
```

Expected: compile error — `cannot find type Config`.

- [ ] **Step 3: Write the implementation**

Prepend to `src-tauri/src/config.rs`:

```rust
use serde::{Deserialize, Serialize};
use std::path::Path;

const FILE_NAME: &str = "config.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub target_id: Option<String>,
    pub target_name: Option<String>,
    pub near_dbm: i16,
    pub away_dbm: i16,
    pub confirm_samples: u8,
    pub grace_seconds: u64,
    pub armed: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            target_id: None,
            target_name: None,
            // Measured: ~-35 dBm at the desk, so -70/-85 leaves a wide
            // hysteresis band to absorb the ~20 dB spontaneous dips.
            near_dbm: -70,
            away_dbm: -85,
            confirm_samples: 3,
            grace_seconds: 10,
            armed: false,
        }
    }
}

impl Config {
    pub fn load(dir: &Path) -> Config {
        // Any failure falls back to defaults, which are disarmed.
        // Never inherit a half-parsed armed state.
        std::fs::read_to_string(dir.join(FILE_NAME))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(FILE_NAME), json).map_err(|e| e.to_string())
    }
}
```

Add to `src-tauri/src/lib.rs`:

```rust
pub mod config;
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd src-tauri && cargo test config 2>&1 | tail -20
```

Expected: `test result: ok. 4 passed`.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/config.rs src-tauri/src/lib.rs
git commit -m "feat: add persisted configuration"
```

---

### Task 6: Monitor loop

Ties sensing to state to lock. The critical property under test: a departure fires the lock **exactly once**, and a failing sensor never fires it at all.

**Files:**
- Create: `src-tauri/src/monitor.rs`
- Modify: `src-tauri/src/lib.rs` (add `pub mod monitor;`)

**Interfaces:**
- Consumes: `crate::ble::{ProximitySource, FakeSource}`, `crate::proximity::{ProximityTracker, Thresholds, Presence}`, `crate::lock::{ScreenLocker, LockBackend}`, `crate::config::Config`
- Produces:
  - `pub struct MonitorStatus { pub presence: Presence, pub rssi: Option<f32>, pub armed: bool, pub error: Option<String> }` (derives `Debug, Clone, serde::Serialize`)
  - `pub async fn run_once(source: &dyn ProximitySource, tracker: &mut ProximityTracker, locker: &dyn ScreenLocker, armed: bool, target_id: &str) -> MonitorStatus`

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/monitor.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ble::FakeSource;
    use crate::lock::{LockBackend, ScreenLocker};
    use crate::proximity::{Presence, ProximityTracker, Thresholds};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RecordingLocker {
        calls: AtomicUsize,
    }
    impl RecordingLocker {
        fn new() -> Self { Self { calls: AtomicUsize::new(0) } }
        fn calls(&self) -> usize { self.calls.load(Ordering::SeqCst) }
    }
    impl ScreenLocker for RecordingLocker {
        fn lock(&self) -> Result<(), String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn backend(&self) -> LockBackend { LockBackend::PrivateApi }
    }

    fn tracker() -> ProximityTracker {
        ProximityTracker::new(Thresholds { near_dbm: -70, away_dbm: -85, confirm_samples: 3 })
    }

    #[tokio::test]
    async fn locks_once_on_departure() {
        let source = FakeSource::new(vec![
            Some(-40), Some(-40), Some(-40),   // establish Near
            Some(-100), Some(-100), Some(-100), // depart
            Some(-100), Some(-100),             // stay gone
        ]);
        let locker = RecordingLocker::new();
        let mut t = tracker();
        for _ in 0..8 {
            run_once(&source, &mut t, &locker, true, "fake-device").await;
        }
        assert_eq!(locker.calls(), 1, "departure must lock exactly once");
    }

    #[tokio::test]
    async fn never_locks_while_disarmed() {
        let source = FakeSource::new(vec![Some(-40), Some(-100), Some(-100), Some(-100)]);
        let locker = RecordingLocker::new();
        let mut t = tracker();
        for _ in 0..6 {
            run_once(&source, &mut t, &locker, false, "fake-device").await;
        }
        assert_eq!(locker.calls(), 0);
    }

    #[tokio::test]
    async fn brief_dip_does_not_lock() {
        let source = FakeSource::new(vec![
            Some(-40), Some(-40), Some(-40),
            Some(-95),            // one spurious dip
            Some(-40), Some(-40),
        ]);
        let locker = RecordingLocker::new();
        let mut t = tracker();
        for _ in 0..6 {
            run_once(&source, &mut t, &locker, true, "fake-device").await;
        }
        assert_eq!(locker.calls(), 0);
    }

    #[tokio::test]
    async fn reports_presence_in_status() {
        let source = FakeSource::new(vec![Some(-40), Some(-40), Some(-40)]);
        let locker = RecordingLocker::new();
        let mut t = tracker();
        let mut last = None;
        for _ in 0..3 {
            last = Some(run_once(&source, &mut t, &locker, true, "fake-device").await);
        }
        assert_eq!(last.unwrap().presence, Presence::Near);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd src-tauri && cargo test monitor 2>&1 | tail -20
```

Expected: compile error — `cannot find function run_once`.

- [ ] **Step 3: Write the implementation**

Prepend to `src-tauri/src/monitor.rs`:

```rust
use crate::ble::ProximitySource;
use crate::lock::ScreenLocker;
use crate::proximity::{Presence, ProximityTracker};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct MonitorStatus {
    pub presence: Presence,
    pub rssi: Option<f32>,
    pub armed: bool,
    /// Set when the sensing chain itself is broken. While armed this must be
    /// surfaced, never swallowed — a silent failure looks identical to "safe".
    pub error: Option<String>,
}

/// One polling round. Locks only on the Near -> Away transition, so a phone
/// that stays away does not re-lock on every subsequent round.
pub async fn run_once(
    source: &dyn ProximitySource,
    tracker: &mut ProximityTracker,
    locker: &dyn ScreenLocker,
    armed: bool,
    target_id: &str,
) -> MonitorStatus {
    let (sample, error) = match source.sample(target_id).await {
        Ok(v) => (v, None),
        // A sensor error is NOT evidence of departure. Report it and do not
        // feed the state machine, so a broken adapter cannot lock the machine.
        Err(e) => {
            return MonitorStatus {
                presence: tracker.state(),
                rssi: tracker.smoothed_rssi(),
                armed,
                error: Some(e),
            }
        }
    };

    let transition = tracker.push(sample);

    if armed && transition == Some(Presence::Away) {
        if let Err(e) = locker.lock() {
            return MonitorStatus {
                presence: tracker.state(),
                rssi: tracker.smoothed_rssi(),
                armed,
                error: Some(format!("lock failed: {e}")),
            };
        }
    }

    MonitorStatus {
        presence: tracker.state(),
        rssi: tracker.smoothed_rssi(),
        armed,
        error,
    }
}
```

Add to `src-tauri/src/lib.rs`:

```rust
pub mod monitor;
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd src-tauri && cargo test monitor 2>&1 | tail -20
```

Expected: `test result: ok. 4 passed`.

- [ ] **Step 5: Run the whole suite**

```bash
cd src-tauri && cargo test 2>&1 | tail -20
```

Expected: 19 tests passing — 7 `proximity`, 2 `lock`, 2 `ble`, 4 `config`, 4 `monitor`.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/monitor.rs src-tauri/src/lib.rs
git commit -m "feat: add monitor loop wiring sensing to lock"
```

---

### Task 7: Menu bar UI

Turns the library into a running menu bar app: tray icon, no Dock icon, device picker, live status.

**Files:**
- Create: `src-tauri/src/commands.rs`
- Create: `src-tauri/Info.plist`
- Modify: `src-tauri/src/lib.rs` (tray setup, state, background task)
- Modify: `src-tauri/tauri.conf.json` (hide window, bundle Info.plist)
- Modify: `src/routes/+page.svelte` (the popover UI)
- Create: `src/routes/+layout.ts`
- Create: `src/lib/api.ts`

**Interfaces:**
- Consumes: everything from Tasks 2–6
- Produces: the running app. Commands: `list_devices() -> Vec<DiscoveredDevice>`, `get_config() -> Config`, `set_config(Config) -> ()`, `get_status() -> MonitorStatus`, `lock_backend() -> LockBackend`

- [ ] **Step 1: Disable SSR so `invoke` works**

Create `src/routes/+layout.ts`:

```ts
export const prerender = true;
export const ssr = false;
```

- [ ] **Step 2: Add the Bluetooth usage description**

macOS requires this string or Bluetooth access is denied in a bundled app. Create `src-tauri/Info.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>NSBluetoothAlwaysUsageDescription</key>
    <string>AwayGuard uses Bluetooth to detect when your iPhone leaves range, so it can lock your screen.</string>
    <key>LSUIElement</key>
    <true/>
</dict>
</plist>
```

- [ ] **Step 3: Hide the main window at startup**

In `src-tauri/tauri.conf.json`, replace the `app.windows` entry with:

```json
"windows": [
  {
    "title": "AwayGuard",
    "width": 360,
    "height": 480,
    "visible": false,
    "resizable": false,
    "decorations": false,
    "skipTaskbar": true,
    "alwaysOnTop": true
  }
]
```

Also set `"productName": "AwayGuard"`.

- [ ] **Step 4: Write the commands**

Create `src-tauri/src/commands.rs`:

```rust
use crate::ble::{DiscoveredDevice, ProximitySource};
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
pub fn set_config(state: State<'_, AppState>, config: Config) -> Result<(), String> {
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
```

- [ ] **Step 5: Wire up state, tray, and the polling task**

Replace the body of `src-tauri/src/lib.rs` below the `pub mod` lines with:

```rust
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
                    .map_err(|e| std::io::Error::other(e))?,
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
```

Add `pub mod commands;` alongside the other module declarations.

- [ ] **Step 6: Write the frontend API wrapper**

Create `src/lib/api.ts`:

```ts
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
```

- [ ] **Step 7: Write the popover UI**

Replace `src/routes/+page.svelte`:

```svelte
<script lang="ts">
  import { onMount } from "svelte";
  import {
    listDevices, getConfig, setConfig, getStatus, lockBackend, onStatus,
    type Config, type DiscoveredDevice, type MonitorStatus, type LockBackend,
  } from "$lib/api";

  let config = $state<Config | null>(null);
  let status = $state<MonitorStatus | null>(null);
  let devices = $state<DiscoveredDevice[]>([]);
  let backend = $state<LockBackend>("unavailable");
  let scanning = $state(false);

  onMount(async () => {
    config = await getConfig();
    status = await getStatus();
    backend = await lockBackend();
    onStatus((s) => (status = s));
  });

  async function scan() {
    scanning = true;
    try { devices = await listDevices(); } finally { scanning = false; }
  }

  async function save() {
    if (config) await setConfig(config);
  }
</script>

<main>
  <header>
    <h1>AwayGuard</h1>
    {#if status}
      <span class="badge {status.presence}">{status.presence}</span>
    {/if}
  </header>

  {#if status?.error}
    <p class="error">⚠ {status.error}</p>
  {/if}

  {#if backend === "screenSaver"}
    <p class="warn">
      Using the screen saver fallback — a lock is not guaranteed unless
      "require password after screen saver begins" is enabled.
    </p>
  {:else if backend === "unavailable"}
    <p class="error">No screen lock mechanism available on this system.</p>
  {/if}

  {#if config}
    <section>
      <button onclick={scan} disabled={scanning}>
        {scanning ? "Scanning…" : "Scan for devices"}
      </button>
      {#if devices.length}
        <select bind:value={config.target_id} onchange={save}>
          <option value={null}>— pick your iPhone —</option>
          {#each devices as d}
            <option value={d.id}>{d.name} ({d.rssi ?? "?"} dBm)</option>
          {/each}
        </select>
      {/if}
    </section>

    <section>
      <label>
        Signal reading
        <strong>{status?.rssi ? status.rssi.toFixed(1) : "—"} dBm</strong>
      </label>
      <label>
        Away below {config.away_dbm} dBm
        <input type="range" min="-100" max="-50" bind:value={config.away_dbm} onchange={save} />
      </label>
      <label>
        Near above {config.near_dbm} dBm
        <input type="range" min="-90" max="-30" bind:value={config.near_dbm} onchange={save} />
      </label>
    </section>

    <section>
      <label class="arm">
        <input type="checkbox" bind:checked={config.armed} onchange={save} />
        Lock screen when I walk away
      </label>
    </section>
  {/if}
</main>

<style>
  main { font: 13px -apple-system, system-ui; padding: 12px; display: grid; gap: 14px; }
  header { display: flex; align-items: center; justify-content: space-between; }
  h1 { font-size: 15px; margin: 0; }
  .badge { font-size: 11px; padding: 2px 8px; border-radius: 999px; background: #eee; }
  .badge.near { background: #d7f5dd; }
  .badge.away { background: #fde2e1; }
  section { display: grid; gap: 8px; }
  label { display: grid; gap: 4px; }
  .arm { display: flex; gap: 8px; align-items: center; }
  .error { color: #b00020; margin: 0; }
  .warn { color: #8a6100; margin: 0; }
</style>
```

- [ ] **Step 8: Run the app and verify end to end**

```bash
pnpm tauri dev
```

Verify each of these:
1. No Dock icon appears; a tray icon does.
2. Clicking the tray icon toggles the popover.
3. "Scan for devices" lists your iPhone by name within a few seconds.
4. Selecting it makes the RSSI readout update and settle on `near`.
5. The lock backend warning does NOT appear (the private API resolves on macOS 26).

Then the real acceptance test: **arm it, take your phone, and walk out of range.** The screen must lock. Then confirm it does not lock again while you stay away.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "feat: add menu bar tray UI and wire the app end to end"
```

---

## Notes for the implementer

- `cargo test` must never lock the screen. `lock()` is deliberately untested; only backend *detection* is.
- If `BleSource::new()` fails at startup, the app currently returns an error from `setup`. If that proves annoying in practice, degrade to an error status instead of refusing to launch — but do not silently continue in an armed state.
- The `grace_seconds` config field is persisted and surfaced but not yet enforced by `run_once`; the lock currently fires immediately on the confirmed transition. Wiring the delay is a natural follow-up, and the confirm-samples mechanism already absorbs brief dips.
