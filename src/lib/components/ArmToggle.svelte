<script lang="ts">
  /*
   * The arm switch. A real `role="switch"` button rather than a styled
   * checkbox, so the label, keyboard behaviour and disabled reason are all
   * one control.
   *
   * `checked` is the user's *intent* only. Whether the Mac is actually
   * protected is the headline's job, and the two are deliberately allowed to
   * disagree — see present.ts.
   */
  let {
    checked,
    disabled = false,
    label,
    /** Drop the visible caption when the row has to share its width with the
     * countdown's cancel button. The switch keeps the same accessible name —
     * only the sighted label goes, and only where the headline right above
     * has already said what is about to happen. */
    labelHidden = false,
    onchange,
  }: {
    checked: boolean;
    disabled?: boolean;
    label: string;
    labelHidden?: boolean;
    onchange: (next: boolean) => void;
  } = $props();
</script>

<button
  type="button"
  role="switch"
  class="row"
  class:bare={labelHidden}
  aria-checked={checked}
  aria-label={labelHidden ? label : undefined}
  {disabled}
  onclick={() => onchange(!checked)}
>
  {#if !labelHidden}<span class="label">{label}</span>{/if}
  <span class="track" class:on={checked}>
    <span class="knob"></span>
  </span>
</button>

<style>
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    width: 100%;
    text-align: left;
  }

  .row.bare {
    width: auto;
    flex: none;
  }

  .row:disabled {
    opacity: 0.45;
  }

  .label {
    font: 400 13px/1.25 var(--sans);
  }

  .track {
    position: relative;
    width: 38px;
    height: 22px;
    flex: none;
    border-radius: 11px;
    background: var(--fill-track);
    border: 0.5px solid var(--fill-border);
    transition: background 180ms ease;
  }

  .track.on {
    background: var(--near-fill);
    border-color: transparent;
    box-shadow: inset 0 0 0 0.5px oklch(0 0 0 / 0.2);
  }

  .knob {
    position: absolute;
    top: 1.5px;
    left: 1.5px;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: oklch(0.72 0.01 250);
    /* Settles once — no bounce. Urgency comes from words, not animation. */
    transition:
      transform 180ms cubic-bezier(0.32, 0.72, 0, 1),
      background 180ms ease;
  }

  .track.on .knob {
    transform: translateX(16px);
    background: oklch(0.99 0 0);
    box-shadow: 0 1px 2px oklch(0 0 0 / 0.3);
  }
</style>
