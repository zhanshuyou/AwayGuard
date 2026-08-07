<script lang="ts">
  import { onMount } from "svelte";
  import {
    cancelPendingLock,
    getConfig,
    getStatus,
    listDevices,
    lockBackend,
    onStatus,
    quit,
    setConfig,
    type Config,
    type DiscoveredDevice,
    type LockBackend,
    type MonitorStatus,
  } from "$lib/api";
  import {
    banner as bannerFor,
    headline as headlineFor,
    shieldState,
    type BannerSpec,
  } from "$lib/present";
  import ArmToggle from "$lib/components/ArmToggle.svelte";
  import Banner from "$lib/components/Banner.svelte";
  import DeviceSection from "$lib/components/DeviceSection.svelte";
  import GraceSlider from "$lib/components/GraceSlider.svelte";
  import PresenceBadge from "$lib/components/PresenceBadge.svelte";
  import ShieldIcon from "$lib/components/ShieldIcon.svelte";
  import SignalMeter from "$lib/components/SignalMeter.svelte";
  import ThresholdBand from "$lib/components/ThresholdBand.svelte";

  let config = $state<Config | null>(null);
  let status = $state<MonitorStatus | null>(null);
  // `null` means "not yet determined". Defaulting this to "unavailable"
  // made a failed startup indistinguishable from a genuine lack of any
  // lock mechanism, so the panel accused the OS of the frontend's problem.
  let backend = $state<LockBackend | null>(null);
  let devices = $state<DiscoveredDevice[]>([]);
  let scanning = $state(false);
  let picking = $state(false);
  let scanError = $state<string | null>(null);
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

  async function save() {
    if (!config) return;
    try {
      await setConfig(config);
      saveError = null;
    } catch (e) {
      // Previously an unawaited/uncaught rejection here vanished silently:
      // the toggle stayed on (it's bound to frontend-local state) while the
      // backend's in-memory config could be stale, so the UI showed "armed"
      // with no guarantee the backend agreed.
      saveError = e instanceof Error ? e.message : String(e);
    }
  }

  async function scan() {
    scanning = true;
    scanError = null;
    try {
      devices = await listDevices();
      picking = true;
    } catch (e) {
      scanError = e instanceof Error ? e.message : String(e);
    } finally {
      scanning = false;
    }
  }

  function pick(device: DiscoveredDevice) {
    if (!config) return;
    config.target_id = device.id;
    config.target_name = device.name;
    picking = false;
    save();
  }

  async function stillHere() {
    try {
      await cancelPendingLock();
      // Don't optimistically clear the countdown: the next status event
      // reports whether it really stopped. Claiming a cancel that didn't
      // land is exactly the kind of lie this panel exists to avoid.
    } catch (e) {
      saveError = e instanceof Error ? e.message : String(e);
    }
  }

  const head = $derived(headlineFor(status, config, backend));
  const shield = $derived(shieldState(status, config, backend));
  const spec = $derived<BannerSpec | null>(
    initError
      ? {
          tone: "danger",
          title: "AwayGuard didn’t start correctly",
          body: `${initError}. Nothing is being monitored.`,
          action: null,
        }
      : bannerFor(status, config, backend),
  );

  const countdown = $derived(status?.grace_remaining ?? null);
  const graceTotal = $derived(config?.grace_seconds ?? 0);
  /** How far through the grace period we are, 0…1. */
  const elapsedFraction = $derived(
    countdown == null || graceTotal === 0 ? 0 : (graceTotal - countdown) / graceTotal,
  );

  /* The popover is a fixed 320 × 420pt, and the composition never changes
     order — status, device, signal, thresholds. What varies is how much
     chrome each section spends, and only as much as the state actually
     needs.

     A banner is the expensive case: it costs roughly a third of the panel,
     so the section headers go, the signal meter folds to one row, and the
     grace slider hands its value to the footer.

     A countdown costs far less, so it only gives up the sensitivity header.
     Over-compacting it would strip controls for no reason — and the
     countdown is exactly when the user might want to adjust grace. */
  const tight = $derived(spec !== null);
  const showGrace = $derived(!tight && !picking);
  const showSensitivityHeader = $derived(!tight && countdown == null);

  const shieldColor = $derived(
    {
      outline: "oklch(0.8 0.012 250)",
      solid: "oklch(0.78 0.06 165)",
      open: "oklch(0.8 0.07 35)",
      slashed: "oklch(0.72 0.12 22)",
    }[shield],
  );

  const graceSummary = $derived(
    !showGrace && config ? `Grace ${config.grace_seconds}s · ` : "",
  );

  const backendLabel = $derived(
    backend === "privateApi"
      ? "Lock backend: system lock"
      : backend === "screenSaver"
        ? "Lock backend: screen saver"
        : backend === "unavailable"
          ? "No lock backend"
          : "Checking lock backend…",
  );
</script>

<main class="popover">
  <header class="titlebar">
    <span class="brand">
      <ShieldIcon state={shield} color={shieldColor} />
      AwayGuard
    </span>
    <PresenceBadge presence={status?.presence ?? "unknown"} />
  </header>

  <div class="body">
    {#if spec}
      <Banner {spec} />
    {/if}

    {#if config}
      <section class="state">
        <div class="head">
          <div class="titles">
            <h1 class="title {head.tone}">
              {#if head.dot}
                <span class="pip {head.tone}" aria-hidden="true"></span>
              {/if}
              {head.title}
            </h1>
            <p class="detail">{head.detail}</p>
          </div>
          {#if countdown != null}
            <!-- The same number as the headline, at a glanceable size.
                 aria-hidden so it isn't announced twice. -->
            <span class="tally num" aria-hidden="true">{countdown}</span>
          {/if}
        </div>

        {#if countdown != null}
          <div
            class="countdown"
            role="progressbar"
            aria-label="Time until this Mac locks"
            aria-valuemin={0}
            aria-valuemax={graceTotal}
            aria-valuenow={graceTotal - countdown}
          >
            <div class="countdown-fill" style:width="{elapsedFraction * 100}%"></div>
          </div>
        {/if}

        <div class="controls">
          {#if countdown != null}
            <button type="button" class="reprieve" onclick={stillHere}>
              Cancel — I’m still here
            </button>
          {/if}
          <ArmToggle
            checked={config.armed}
            disabled={!config.target_id}
            label="Lock screen when I walk away"
            labelHidden={countdown != null}
            onchange={(next) => {
              config!.armed = next;
              save();
            }}
          />
        </div>

        {#if !config.target_id}
          <p class="hint">Select your iPhone first.</p>
        {/if}
        {#if saveError}
          <p class="hint danger">Couldn’t save that change: {saveError}</p>
        {/if}
      </section>

      <DeviceSection
        {config}
        {status}
        {devices}
        {scanning}
        {scanError}
        {picking}
        compact={tight}
        onscan={scan}
        onpick={pick}
        ondismiss={() => (picking = false)}
      />

      <!-- While the scan list is open the user is mid-task picking a device;
           the tuning controls below it are about a device they haven't chosen
           yet, so the list gets the room instead. -->
      {#if !picking}
        <!-- With no device there is no signal to plot and nothing to say
             about its freshness, so the meter collapses to its one dimmed
             row rather than padding out two. -->
        <SignalMeter {status} {config} compact={tight || !config.target_id} />

        <ThresholdBand
          away={config.away_dbm}
          near={config.near_dbm}
          live={status?.rssi ?? null}
          disabled={!config.target_id}
          showHeader={showSensitivityHeader}
          onchange={({ away, near }) => {
            config!.away_dbm = away;
            config!.near_dbm = near;
          }}
          oncommit={save}
        />

        {#if showGrace}
          <GraceSlider
            seconds={config.grace_seconds}
            disabled={!config.target_id}
            onchange={(next) => (config!.grace_seconds = next)}
            oncommit={save}
          />
        {/if}
      {/if}
    {/if}

    <!-- Omitted while picking, so the scan list rather than empty space
         takes the room the hidden sections freed up. -->
    {#if !picking}
      <div class="filler"></div>
    {/if}
  </div>

  <footer class="statusbar">
    <!-- The grace value lives here whenever its slider has been compacted
         away, so the popover never stops stating it. -->
    <span class="foot" class:degraded={backend === "screenSaver" || backend === "unavailable"}>
      {graceSummary}{backendLabel}
    </span>
    <button type="button" class="foot quit" onclick={() => quit()}>Quit</button>
  </footer>
</main>

<style>
  .popover {
    display: flex;
    flex-direction: column;
    width: 100vw;
    height: 100vh;
    border-radius: var(--radius);
    overflow: hidden;
    background: var(--surface);
    /* Layers over the window's native vibrancy so the popover reads as
       macOS material rather than a flat dark rectangle. */
    backdrop-filter: blur(30px);
    border: 0.5px solid var(--edge);
  }

  .titlebar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex: none;
    padding: 9px var(--pad);
    border-bottom: 0.5px solid var(--hairline);
    /* The one place the window can be dragged, since it has no title bar. */
    -webkit-app-region: drag;
  }

  .titlebar :global(button) {
    -webkit-app-region: no-drag;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    font: 590 13px/1 var(--sans);
    letter-spacing: -0.005em;
  }

  /* Fixed chrome above and below; the middle gives way if a future state
     ever needs more room than the compaction rules free up. */
  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
  }

  .filler {
    flex: 1;
  }

  .state {
    display: flex;
    flex-direction: column;
    gap: 9px;
    padding: 11px var(--pad) 12px;
  }

  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 10px;
  }

  .titles {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }

  .title {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0;
    font: 600 19px/1.1 var(--display);
    letter-spacing: -0.02em;
    color: var(--text-strong);
  }

  .title.away {
    color: oklch(0.92 0.03 35);
  }

  .title.danger {
    color: oklch(0.9 0.02 22);
  }

  .pip {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex: none;
    background: var(--near);
    box-shadow: 0 0 0 4px oklch(0.76 0.08 165 / 0.16);
  }

  .pip.warn {
    background: var(--warn);
    box-shadow: 0 0 0 4px oklch(0.78 0.1 80 / 0.16);
  }

  .detail {
    margin: 0;
    font: 400 12px/1.4 var(--sans);
    color: var(--text-muted);
    text-wrap: pretty;
  }

  .tally {
    flex: none;
    font: 500 27px/1 var(--mono);
    color: oklch(0.84 0.08 35);
  }

  .countdown {
    height: 3px;
    border-radius: 2px;
    background: var(--fill-track);
    overflow: hidden;
  }

  .countdown-fill {
    height: 100%;
    background: oklch(0.72 0.09 35);
    transition: width 300ms linear;
  }

  .controls {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }

  .reprieve {
    flex: none;
    padding: 4px 11px;
    border-radius: 6px;
    background: var(--fill-strong);
    border: 0.5px solid oklch(1 0 0 / 0.12);
    font: 500 12px/1.3 var(--sans);
  }

  .hint {
    margin: 0;
    font: 400 11.5px/1.35 var(--sans);
    color: oklch(0.74 0.05 80);
    text-wrap: pretty;
  }

  .hint.danger {
    color: var(--danger-text);
  }

  .statusbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    flex: none;
    padding: 8px var(--pad);
    border-top: 0.5px solid var(--hairline);
  }

  .foot {
    font: 400 11px/1.2 var(--sans);
    color: var(--text-faint);
  }

  .foot.degraded {
    color: oklch(0.7 0.05 80);
  }

  .quit:hover {
    color: var(--text-muted);
  }
</style>
