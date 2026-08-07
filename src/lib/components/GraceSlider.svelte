<script lang="ts">
  /*
   * How long the countdown runs before the screen locks. Zero is allowed and
   * means "lock the moment departure is confirmed" — worth keeping reachable,
   * but the caption says so plainly rather than showing a bare "0s".
   */
  let {
    seconds,
    max = 60,
    disabled = false,
    onchange,
    oncommit,
  }: {
    seconds: number;
    max?: number;
    disabled?: boolean;
    onchange: (next: number) => void;
    oncommit: () => void;
  } = $props();

  /** Matches .knob below; see ThresholdBand for why the fill needs the inset. */
  const THUMB = 14;
  const fraction = $derived(max === 0 ? 0 : seconds / max);
  const knobX = $derived(
    `calc(${(fraction * 100).toFixed(3)}% + ${((0.5 - fraction) * THUMB).toFixed(2)}px)`,
  );
</script>

<section class="grace" class:disabled>
  <label class="label" for="grace-range">Grace</label>
  <div class="rail">
    <div class="track" aria-hidden="true"></div>
    <div class="fill" aria-hidden="true" style:right="calc(100% - {knobX})"></div>
    <input
      id="grace-range"
      type="range"
      min="0"
      {max}
      step="1"
      value={seconds}
      {disabled}
      aria-valuetext={seconds === 0 ? "Lock immediately" : `${seconds} seconds`}
      oninput={(e) => onchange(e.currentTarget.valueAsNumber)}
      onchange={oncommit}
    />
  </div>
  <span class="value num">{seconds === 0 ? "none" : `${seconds}s`}</span>
</section>

<style>
  .grace {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 9px var(--pad);
    border-top: 0.5px solid var(--hairline);
  }

  .disabled {
    opacity: 0.4;
  }

  .label {
    flex: none;
    font: 400 12px/1 var(--sans);
    color: oklch(0.8 0.01 250);
  }

  .rail {
    position: relative;
    flex: 1;
    height: 14px;
  }

  .track,
  .fill {
    position: absolute;
    top: 5px;
    height: 4px;
    border-radius: 2px;
  }

  .track {
    left: 0;
    right: 0;
    background: var(--fill-track);
  }

  .fill {
    left: 0;
    background: oklch(0.78 0.01 250);
  }

  input {
    position: absolute;
    inset: 0;
    width: 100%;
    margin: 0;
    -webkit-appearance: none;
    appearance: none;
    background: transparent;
  }

  input::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--thumb);
    border: 0.5px solid var(--thumb-edge);
  }

  .value {
    flex: none;
    min-width: 30px;
    text-align: right;
    font: 500 11.5px/1 var(--mono);
    color: oklch(0.88 0.01 250);
  }
</style>
