<script lang="ts">
  import { MIN_GAP_DBM, RSSI_CEIL, RSSI_FLOOR, enforceGap, pct } from "$lib/present";

  /*
   * The dual-handle sensitivity control, and the design's central idea: the
   * hatched span between the handles is the hysteresis band. Inside it
   * nothing changes state, so a phone resting at the edge of range can't
   * toggle the lock every few seconds.
   *
   * Two real range inputs stacked over a painted track — the handles stay
   * keyboard-operable and announce themselves, which a div-and-pointer
   * implementation would throw away.
   */
  let {
    away,
    near,
    live = null,
    disabled = false,
    showHeader = true,
    onchange,
    oncommit,
  }: {
    away: number;
    near: number;
    /** Current smoothed reading, drawn as the white tick. */
    live?: number | null;
    disabled?: boolean;
    /** Dropped when a banner or countdown needs the vertical space. The band
     * and its two dBm labels always stay — they are the numbers behind the
     * claim; only the section heading is expendable. */
    showHeader?: boolean;
    onchange: (next: { away: number; near: number }) => void;
    oncommit: () => void;
  } = $props();

  /** Must match .thumb width below. A native range thumb's centre travels
   * `thumb/2 … track − thumb/2`, not `0 … track`, so the painted zones have
   * to use the same inset or the band edges drift away from the handles. */
  const THUMB = 15;

  function x(dbm: number): string {
    const f = pct(dbm) / 100;
    return `calc(${(f * 100).toFixed(3)}% + ${((0.5 - f) * THUMB).toFixed(2)}px)`;
  }

  function move(moved: "away" | "near", raw: number) {
    onchange(
      enforceGap(
        moved === "away" ? raw : away,
        moved === "near" ? raw : near,
        moved,
      ),
    );
  }

  const band = $derived(near - away);
</script>

<section class="sensitivity" class:disabled>
  {#if showHeader}
    <header>
      <span class="eyebrow">Sensitivity</span>
      <span class="band-size num" class:tight={band <= MIN_GAP_DBM}>{band} dB band</span>
    </header>
  {/if}

  <div class="rail">
    <div class="track" aria-hidden="true"></div>
    <div class="zone away" aria-hidden="true" style:right="calc(100% - {x(away)})"></div>
    <div class="zone hyst" aria-hidden="true" style:left={x(away)} style:right="calc(100% - {x(near)})"></div>
    <div class="zone near" aria-hidden="true" style:left={x(near)}></div>
    {#if live != null}
      <div class="tick" aria-hidden="true" style:left={x(live)}></div>
    {/if}

    <input
      type="range"
      class="handle"
      min={RSSI_FLOOR}
      max={RSSI_CEIL}
      step="1"
      value={away}
      {disabled}
      aria-label="Away threshold"
      aria-valuetext="Away below {away} dBm"
      oninput={(e) => move("away", e.currentTarget.valueAsNumber)}
      onchange={oncommit}
    />
    <input
      type="range"
      class="handle"
      min={RSSI_FLOOR}
      max={RSSI_CEIL}
      step="1"
      value={near}
      {disabled}
      aria-label="Near threshold"
      aria-valuetext="Near above {near} dBm"
      oninput={(e) => move("near", e.currentTarget.valueAsNumber)}
      onchange={oncommit}
    />
  </div>

  <footer>
    <span>Away below <span class="num">−{Math.abs(away)}</span></span>
    <span>Near above <span class="num">−{Math.abs(near)}</span></span>
  </footer>
</section>

<style>
  .sensitivity {
    display: flex;
    flex-direction: column;
    gap: 7px;
    padding: 9px var(--pad);
    border-top: 0.5px solid var(--hairline);
  }

  .disabled {
    opacity: 0.4;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .band-size {
    font: 500 10.5px/1 var(--mono);
    color: oklch(0.7 0.04 165);
  }

  /* At the floor the band can't shrink further; say so rather than letting
     the drag feel broken. */
  .band-size.tight {
    color: oklch(0.76 0.06 80);
  }

  .rail {
    position: relative;
    height: 20px;
  }

  .track,
  .zone {
    position: absolute;
    top: 9px;
    height: 6px;
  }

  .track {
    left: 0;
    right: 0;
    border-radius: 3px;
    background: var(--fill-track);
  }

  .zone.away {
    left: 0;
    border-radius: 3px 0 0 3px;
    background: oklch(0.66 0.07 35);
  }

  /* Hatching, not a flat fill: this span is "no decision", visibly unlike
     the two zones that do decide. */
  .zone.hyst {
    background: repeating-linear-gradient(
      115deg,
      oklch(1 0 0 / 0.16) 0 3px,
      oklch(1 0 0 / 0.04) 3px 7px
    );
  }

  .zone.near {
    right: 0;
    border-radius: 0 3px 3px 0;
    background: oklch(0.66 0.08 165);
  }

  .tick {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 1.5px;
    margin-left: -0.75px;
    background: oklch(0.92 0.01 250);
    transition: left 240ms ease;
  }

  /* Both inputs span the full rail so either handle can be grabbed anywhere
     it happens to be; only the thumbs take pointer events, so the one
     underneath is never shadowed by the other's invisible track. */
  .handle {
    position: absolute;
    inset: 0;
    width: 100%;
    margin: 0;
    -webkit-appearance: none;
    appearance: none;
    background: transparent;
    pointer-events: none;
  }

  .handle::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    pointer-events: auto;
    width: 15px;
    height: 15px;
    border-radius: 50%;
    background: var(--thumb);
    border: 0.5px solid var(--thumb-edge);
    box-shadow: 0 1px 2px oklch(0 0 0 / 0.3);
  }

  .handle:disabled::-webkit-slider-thumb {
    pointer-events: none;
    background: oklch(0.9 0.006 250);
    box-shadow: none;
  }

  footer {
    display: flex;
    justify-content: space-between;
    font: 400 11px/1 var(--sans);
    color: oklch(0.76 0.01 250);
  }
</style>
