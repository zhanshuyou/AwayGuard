<script lang="ts">
  import type { ShieldState } from "$lib/present";

  /*
   * The AwayGuard mark: a shield whose *shape* is the state.
   *
   *   outline  disarmed        solid  armed and near
   *   open     away, pending   slashed  sensing broken
   *
   * Drawn with a border-radius pair rather than an SVG path so it stays
   * crisp at 13px and inherits colour, matching the template assets the
   * design specifies for the menu bar itself.
   */
  let {
    state = "outline" as ShieldState,
    width = 13,
    color = "currentColor",
    stroke = 1.5,
  }: {
    state?: ShieldState;
    width?: number;
    color?: string;
    stroke?: number;
  } = $props();

  const height = $derived(Math.round((width * 15) / 13));
</script>

<span
  class="shield {state}"
  style:--w="{width}px"
  style:--h="{height}px"
  style:--c={color}
  style:--s="{stroke}px"
>
  {#if state === "slashed"}<span class="slash"></span>{/if}
</span>

<style>
  .shield {
    position: relative;
    display: inline-block;
    width: var(--w);
    height: var(--h);
    border-radius: 2px 2px 60% 60% / 2px 2px 80% 80%;
    flex: none;
  }

  .outline,
  .slashed {
    border: var(--s) solid var(--c);
  }

  .solid {
    background: var(--c);
  }

  /* Away: the shield opens at the top — protection is lapsing. */
  .open {
    border: var(--s) solid var(--c);
    border-top: none;
    border-radius: 0 0 60% 60% / 0 0 80% 80%;
  }

  .slash {
    position: absolute;
    top: 50%;
    left: 50%;
    width: calc(var(--w) * 1.5);
    height: var(--s);
    background: var(--c);
    transform: translate(-50%, -50%) rotate(-38deg);
  }
</style>
