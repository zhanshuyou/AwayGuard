<script lang="ts">
  import { onMount } from "svelte";
  import {
    listDevices, getConfig, setConfig, getStatus, lockBackend, onStatus,
    type Config, type DiscoveredDevice, type MonitorStatus, type LockBackend,
  } from "$lib/api";

  // Must match the server-side floor in src-tauri/src/commands.rs
  // (normalize_thresholds's MIN_THRESHOLD_GAP_DBM). Keeping near_dbm >
  // away_dbm here is a UX nicety -- the backend re-enforces it on every
  // set_config call regardless, since it is the one that must never fail
  // open.
  const MIN_GAP_DBM = 1;

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

  // The away and near sliders have overlapping legal ranges (-100..-50 and
  // -90..-30). Without this, dragging one past the other produces
  // near_dbm <= away_dbm, which makes the "near" branch of the proximity
  // state machine win forever -- the app looks armed and healthy but can
  // never lock. Clamp each slider against its sibling as it moves so the
  // crossed state is simply unreachable from the UI.
  function clampAway() {
    if (!config) return;
    if (config.away_dbm >= config.near_dbm) {
      config.away_dbm = config.near_dbm - MIN_GAP_DBM;
    }
  }

  function clampNear() {
    if (!config) return;
    if (config.near_dbm <= config.away_dbm) {
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
        <input
          type="range"
          min="-100"
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
