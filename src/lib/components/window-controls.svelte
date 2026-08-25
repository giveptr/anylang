<script lang="ts">
  import { Copy, Minus, Square, X } from '@lucide/svelte';
  import { close, frame, minimize, toggle } from '$lib/frame.svelte';
</script>

<div class="flex h-full items-stretch">
  <button
    type="button"
    class="chrome-button h-full w-11"
    onclick={minimize}
    aria-label="Minimize"
  >
    <Minus class="size-4" />
  </button>

  <button
    type="button"
    class="chrome-button h-full w-11"
    onclick={toggle}
    aria-label={frame.maximized ? 'Restore' : 'Maximize'}
  >
    {#if frame.maximized}
      <Copy class="size-3.5" />
    {:else}
      <Square class="size-3.5" />
    {/if}
  </button>

  <button
    type="button"
    class="grid h-full w-11 place-items-center text-ink-soft transition-colors hover:bg-close hover:text-on-accent"
    onclick={close}
    aria-label="Close"
  >
    <X class="size-4" />
  </button>
</div>
