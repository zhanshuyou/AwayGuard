<script lang="ts">
  import { onMount } from "svelte";
  import {
    listDevices, getConfig, setConfig, getStatus, lockBackend, onStatus,
    type Config, type DiscoveredDevice, type MonitorStatus, type LockBackend,
  } from "$lib/api";

  // Must match the server-side floor in src-tauri/src/config.rs
  // (Config::normalize_thresholds's MIN_THRESHOLD_GAP_DBM). Keeping
  // near_dbm > away_dbm here is a UX nicety -- the backend re-enforces it
  // on every set_config call regardless, since it is the one that must
  // never fail open. Measured RSSI spread on real hardware is ~20 dB, so
  // this must be a genuinely usable band, not just a non-empty one -- a
  // 1 dB gap yields conclusive evidence on effectively every poll.
  const MIN_GAP_DBM = 12;

  let config = $state<Config | null>(null);
  let status = $state<MonitorStatus | null>(null);
  let devices = $state<DiscoveredDevice[]>([]);
  // `null` means "not yet determined". Defaulting this to "unavailable"
  // made a failed startup indistinguishable from a genuine lack of any
  // lock mechanism, so the panel accused the OS of the frontend's problem.
  let backend = $state<LockBackend | null>(null);
  let scanning = $state(false);
  let saveError = $state<string | null>(null);
  let initError = $state<string | null>(null);

  onMount(async () => {
    // Previously uncaught: a rejection here left `config` null, so the whole
    // panel body silently vanished and the untouched `backend` default
    // rendered a misleading "no lock mechanism" error. Report what actually
    // failed, and name the call that failed it.
    let step = "get_config";
    try {
      config = await getConfig();
      step = "get_status";
      status = await getStatus();
      step = "lock_backend";
      backend = await lockBackend();
      onStatus((s) => (status = s));
    } catch (e) {
      initError = `startup failed at ${step}: ${e}`;
    }
  });

  async function scan() {
    scanning = true;
    try { devices = await listDevices(); } finally { scanning = false; }
  }

  async function save() {
    if (!config) return;
    try {
      await setConfig(config);
      saveError = null;
    } catch (e) {
      // Previously an unawaited/uncaught rejection here vanished silently:
      // the checkbox stayed ticked (it's bound to frontend-local state)
      // while the backend's in-memory config could be stale, so the UI
      // showed "armed" with no guarantee the backend agreed.
      saveError = e instanceof Error ? e.message : String(e);
    }
  }

  // The away and near sliders have overlapping legal ranges (-105..-50 and
  // -90..-30). These are floor checks (>= near - MIN_GAP_DBM), not just
  // crossing checks (>= near) -- dragging Away up to one notch below Near
  // (e.g. near=-70, away=-71) never crosses, but is still a 1 dB band
  // against ~20 dB of measured RSSI noise, and previously slipped through
  // untouched because it never crossed. Clamp each slider against its
  // sibling as it moves so a sub-floor band is simply unreachable from the
  // UI (the backend re-enforces the same floor regardless, since it is the
  // one that must never fail open).
  function clampAway() {
    if (!config) return;
    if (config.away_dbm >= config.near_dbm - MIN_GAP_DBM) {
      config.away_dbm = config.near_dbm - MIN_GAP_DBM;
    }
  }

  function clampNear() {
    if (!config) return;
    if (config.near_dbm <= config.away_dbm + MIN_GAP_DBM) {
      config.near_dbm = config.away_dbm + MIN_GAP_DBM;
    }
  }
</script>

<main>
  <header>
    <h1>AwayGuard</h1>
    {#if status}
      <span class="badge {status.presence}">{status.presence}</span>
    {/if}
  </header>

  {#if status}
    <!-- Distinct from the "Lock screen when I walk away" checkbox below,
         which is only the user's desired/local state. This reflects what
         the backend is actually doing, so a broken config/polling chain
         (checkbox ticked, backend not receiving it, or the monitor loop
         dead) is visible instead of silently invisible. -->
    <p class="armed-state {status.armed ? 'on' : 'off'}">
      Backend is {status.armed ? "armed" : "not armed"}
    </p>
  {/if}

  {#if status?.error}
    <p class="error">⚠ {status.error}</p>
  {/if}

  {#if saveError}
    <p class="error">⚠ Failed to save settings: {saveError}</p>
  {/if}

  {#if backend === "screenSaver"}
    <p class="warn">
      Using the screen saver fallback — a lock is not guaranteed unless
      "require password after screen saver begins" is enabled.
    </p>
  {:else if backend === "unavailable"}
    <p class="error">No screen lock mechanism available on this system.</p>
  {/if}

  {#if initError}
    <p class="error">⚠ {initError}</p>
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
        <input
          type="range"
          min="-105"
          max="-50"
          bind:value={config.away_dbm}
          oninput={clampAway}
          onchange={save}
        />
      </label>
      <label>
        Near above {config.near_dbm} dBm
        <input
          type="range"
          min="-90"
          max="-30"
          bind:value={config.near_dbm}
          oninput={clampNear}
          onchange={save}
        />
      </label>
      <label>
        Grace period before locking: {config.grace_seconds}s
        <input
          type="range"
          min="0"
          max="60"
          bind:value={config.grace_seconds}
          onchange={save}
        />
      </label>
    </section>

    <section>
      <label class="arm">
        <input
          type="checkbox"
          bind:checked={config.armed}
          onchange={save}
          disabled={!config.target_id}
        />
        Lock screen when I walk away
      </label>
      {#if !config.target_id}
        <p class="hint">Select a device above before arming.</p>
      {/if}
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
  .hint { color: #666; margin: 0; font-size: 11px; }
  .armed-state { margin: 0; font-size: 11px; }
  .armed-state.on { color: #1a7a34; }
  .armed-state.off { color: #666; }
</style>
