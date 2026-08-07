<script lang="ts">
  import type { Config, DiscoveredDevice, MonitorStatus } from "$lib/api";

  let {
    config,
    status,
    devices,
    scanning,
    scanError,
    picking,
    /** Drop the section header and fold the scan control into the device row
     * itself, when a banner or countdown above needs the vertical space. */
    compact = false,
    onscan,
    onpick,
    ondismiss,
  }: {
    config: Config;
    status: MonitorStatus | null;
    devices: DiscoveredDevice[];
    scanning: boolean;
    scanError: string | null;
    picking: boolean;
    compact?: boolean;
    onscan: () => void;
    onpick: (device: DiscoveredDevice) => void;
    ondismiss: () => void;
  } = $props();

  /** The live reading for the *selected* device, coloured by what the
   * monitor concluded from it — the number and the verdict come from the
   * same place, so they can't contradict each other. */
  const reading = $derived(
    status?.rssi == null ? null : `−${Math.abs(status.rssi).toFixed(1)} dBm`,
  );
</script>

{#snippet scanButton(label: string)}
  <button type="button" class="scan" onclick={onscan} disabled={scanning}>
    {scanning ? "Scanning…" : label}
  </button>
{/snippet}

<section class="device" class:compact class:picking>
  {#if !compact || picking}
    <header>
      <span class="eyebrow">Device</span>
      {@render scanButton("Scan for devices")}
    </header>
  {/if}

  {#if picking}
    {#if devices.length}
      <ul class="results">
        {#each devices as device (device.id)}
          <li>
            <button
              type="button"
              class="row result"
              class:current={device.id === config.target_id}
              onclick={() => onpick(device)}
            >
              <span class="ident">
                <span class="phone" aria-hidden="true"></span>
                <span class="name">{device.name}</span>
              </span>
              <span class="rssi num">
                {device.rssi == null ? "—" : `−${Math.abs(device.rssi)}`}
              </span>
            </button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="row empty note">No named devices found. Wake your phone and scan again.</p>
    {/if}
    <button type="button" class="dismiss" onclick={ondismiss}>Done</button>
  {:else if config.target_id}
    <div class="row selected">
      <span class="ident">
        <span class="phone" aria-hidden="true"></span>
        <span class="name">{config.target_name ?? "Selected device"}</span>
      </span>
      <span class="trailing">
        <span class="rssi num {status?.presence ?? 'unknown'}">{reading ?? "—"}</span>
        {#if compact}{@render scanButton("Scan")}{/if}
      </span>
    </div>
  {:else}
    <div class="row empty">
      <span>No device selected</span>
      <span class="trailing">
        <span class="rssi num">—</span>
        {#if compact}{@render scanButton("Scan")}{/if}
      </span>
    </div>
  {/if}

  {#if scanError}
    <p class="error">Scan failed: {scanError}</p>
  {/if}
</section>

<style>
  .device {
    display: flex;
    flex-direction: column;
    gap: 7px;
    padding: 9px var(--pad);
    border-top: 0.5px solid var(--hairline);
  }

  .device.compact {
    padding: 8px var(--pad);
  }

  .device.picking {
    flex: 1;
    min-height: 0;
  }

  .device.compact .row {
    padding: 8px 11px;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .scan {
    padding: 3px 10px;
    border-radius: 6px;
    background: var(--fill);
    border: 0.5px solid var(--fill-border);
    font: 500 11.5px/1.3 var(--sans);
    color: oklch(0.84 0.01 250);
  }

  .scan:disabled {
    color: var(--text-faint);
  }

  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    width: 100%;
    padding: 9px 11px;
    border-radius: 8px;
    text-align: left;
  }

  .selected,
  .result {
    background: var(--fill-subtle);
    border: 0.5px solid var(--fill-border);
  }

  .result.current {
    border-color: var(--near-edge);
  }

  .empty {
    border: 0.5px dashed oklch(1 0 0 / 0.2);
    font: 400 12.5px/1.35 var(--sans);
    color: var(--text-dim);
  }

  .note {
    margin: 0;
    display: block;
    text-wrap: pretty;
  }

  .ident {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }

  .phone {
    flex: none;
    width: 9px;
    height: 14px;
    border: 1px solid oklch(0.8 0.01 250);
    border-radius: 2px;
  }

  .name {
    font: 500 12.5px/1 var(--sans);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .trailing {
    display: flex;
    align-items: center;
    gap: 9px;
    flex: none;
  }

  .rssi {
    flex: none;
    font: 500 11.5px/1 var(--mono);
    color: var(--text-dim);
  }

  .rssi.near {
    color: var(--near-text);
  }

  .rssi.away {
    color: oklch(0.85 0.07 35);
  }

  .results {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    /* Takes the room the hidden tuning sections freed up, and scrolls past
       that. A row clipped in half with space going spare below it reads as a
       bug, not as "there is more". */
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
  }

  .dismiss {
    align-self: flex-end;
    font: 500 11px/1 var(--sans);
    color: var(--text-dim);
  }

  .error {
    margin: 0;
    font: 400 11px/1.35 var(--sans);
    color: var(--danger-text);
    text-wrap: pretty;
  }
</style>
