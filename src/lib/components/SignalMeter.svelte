<script lang="ts">
  import type { Config, MonitorStatus } from "$lib/api";
  import { pct, signalCaption, signalCaptionShort, signalWord } from "$lib/present";

  let {
    status,
    config,
    /** Collapse to a single row when a banner or countdown needs the space.
     * The bar and the freshness stay; the poll cadence is what goes. */
    compact = false,
  }: { status: MonitorStatus | null; config: Config; compact?: boolean } = $props();

  const rssi = $derived(status?.rssi ?? null);
  const word = $derived(rssi == null ? null : signalWord(rssi, config));
</script>

<!-- Dimmed rather than hidden when there is no reading: the meter is part of
     the popover's fixed skeleton, so states don't reflow past each other. -->
<section class="meter" class:idle={rssi == null} class:compact>
  <div class="line">
    <div class="track">
      {#if rssi != null}
        <div class="fill {word}" style:width="{pct(rssi)}%"></div>
      {/if}
    </div>
    <span class="word" class:num={!compact}>
      {compact ? signalCaptionShort(status) : (word ?? "— dBm")}
    </span>
  </div>
  {#if !compact}
    <p class="caption">{signalCaption(status)}</p>
  {/if}
</section>

<style>
  .meter {
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding: 9px var(--pad);
    border-top: 0.5px solid var(--hairline);
  }

  .meter.compact {
    padding: 8px var(--pad);
  }

  .idle {
    opacity: 0.4;
  }

  .line {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .track {
    position: relative;
    flex: 1;
    height: 4px;
    border-radius: 2px;
    background: var(--fill-track);
    overflow: hidden;
  }

  .fill {
    position: absolute;
    inset: 0 auto 0 0;
    border-radius: 2px;
    background: oklch(0.72 0.07 165);
    transition: width 240ms ease;
  }

  .fill.weak {
    background: var(--away-fill);
  }

  .fill.steady {
    background: oklch(0.78 0.01 250);
  }

  .word {
    flex: none;
    font: 500 11px/1 var(--mono);
    color: oklch(0.86 0.01 250);
  }

  .compact .word {
    font: 400 11px/1 var(--sans);
    color: var(--text-dim);
  }

  .caption {
    margin: 0;
    font: 400 11px/1.25 var(--sans);
    color: var(--text-dim);
  }
</style>
