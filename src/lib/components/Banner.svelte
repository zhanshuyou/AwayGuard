<script lang="ts">
  import type { BannerSpec } from "$lib/present";
  import { openSettings } from "$lib/api";

  let { spec }: { spec: BannerSpec } = $props();

  let actionError = $state<string | null>(null);

  async function act() {
    if (!spec.action) return;
    try {
      await openSettings(spec.action.pane);
      actionError = null;
    } catch (e) {
      // A banner whose fix-it button silently does nothing is worse than no
      // button, so say when it failed and leave the user the manual route.
      actionError = e instanceof Error ? e.message : String(e);
    }
  }
</script>

<!-- role="alert" so the reason protection is degraded is announced the
     moment it appears, not only when the user goes looking. -->
<div class="banner {spec.tone}" role="alert">
  <span class="mark" aria-hidden="true"></span>
  <div class="body">
    <div class="title">{spec.title}</div>
    <p class="detail">{spec.body}</p>
    {#if spec.action}
      <button type="button" class="action" onclick={act}>{spec.action.label}</button>
    {/if}
    {#if actionError}
      <p class="detail">Couldn’t open Settings ({actionError}). Open it from the Apple menu.</p>
    {/if}
  </div>
</div>

<style>
  .banner {
    display: flex;
    gap: 9px;
    margin: 9px 12px 0;
    padding: 9px 11px;
    border-radius: 8px;
    border: 0.5px solid;
  }

  .body {
    display: flex;
    flex-direction: column;
    gap: 5px;
    min-width: 0;
  }

  .title {
    font: 590 12px/1.3 var(--sans);
  }

  .detail {
    margin: 0;
    font: 400 11.5px/1.4 var(--sans);
    text-wrap: pretty;
  }

  .action {
    align-self: flex-start;
    font: 500 11.5px/1.3 var(--sans);
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  .mark {
    flex: none;
    width: 14px;
    height: 14px;
    margin-top: 1px;
  }

  .danger {
    background: var(--danger-bg);
    border-color: var(--danger-edge);
  }
  .danger .title {
    color: var(--danger-text);
  }
  .danger .detail {
    color: var(--danger-body);
  }
  .danger .action {
    color: oklch(0.9 0.04 22);
  }
  .danger .mark {
    background: var(--danger);
    border-radius: 50%;
  }

  .warn {
    background: var(--warn-bg);
    border-color: var(--warn-edge);
  }
  .warn .title {
    color: var(--warn-text);
  }
  .warn .detail {
    color: var(--warn-body);
  }
  .warn .action {
    color: oklch(0.9 0.05 85);
  }
  /* A triangle, so error and warning differ in shape and not only in hue. */
  .warn .mark {
    width: 12px;
    height: 12px;
    margin-top: 2px;
    background: var(--warn);
    clip-path: polygon(50% 0, 100% 100%, 0 100%);
  }
</style>
