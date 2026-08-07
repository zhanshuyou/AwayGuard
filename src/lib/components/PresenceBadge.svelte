<script lang="ts">
  import type { Presence } from "$lib/api";
  import { presenceLabel } from "$lib/present";

  let { presence }: { presence: Presence } = $props();
</script>

<!-- Reads as a sentence to a screen reader, where the dot's colour is
     invisible: "Phone proximity: Near". -->
<span class="badge {presence}">
  <span class="dot" aria-hidden="true"></span>
  <span class="sr">Phone proximity:</span>
  {presenceLabel(presence)}
</span>

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 3px 9px 3px 7px;
    border-radius: 11px;
    font: 500 11px/1 var(--sans);
    background: var(--fill);
    border: 0.5px solid var(--fill-border);
    color: oklch(0.78 0.01 250);
    /* Crossfade between states — the badge is the only thing that moves. */
    transition:
      background 200ms ease,
      border-color 200ms ease,
      color 200ms ease;
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: oklch(0.7 0.012 250);
    transition: background 200ms ease;
  }

  .near {
    background: var(--near-bg);
    border-color: var(--near-edge);
    color: var(--near-text);
  }
  .near .dot {
    background: var(--near);
  }

  .away {
    background: var(--away-bg);
    border-color: var(--away-edge);
    color: var(--away-text);
  }
  .away .dot {
    background: var(--away);
  }

  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
</style>
