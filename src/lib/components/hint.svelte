<script lang="ts">
  import { tick, type Snippet } from 'svelte';

  const GAP = 6;
  const WAIT = 350;

  let {
    text,
    children,
  }: {
    text: string;
    children: Snippet;
  } = $props();

  let anchor = $state<HTMLElement | null>(null);
  let bubble = $state<HTMLElement | null>(null);
  let shown = $state(false);
  let at = $state({ x: -9999, y: -9999 });
  let timer: ReturnType<typeof setTimeout> | null = null;

  async function place() {
    const anchored = (
      anchor?.firstElementChild ?? anchor
    )?.getBoundingClientRect();
    if (!anchored) return;

    shown = true;
    await tick();

    const size = bubble?.getBoundingClientRect();
    if (!size) return;

    at = {
      x: Math.min(
        Math.max(GAP, anchored.left + anchored.width / 2 - size.width / 2),
        window.innerWidth - size.width - GAP,
      ),
      y: anchored.bottom + GAP,
    };
  }

  function show() {
    if (shown || timer) return;
    timer = setTimeout(place, WAIT);
  }

  function hide() {
    if (timer) clearTimeout(timer);
    timer = null;
    shown = false;
    at = { x: -9999, y: -9999 };
  }
</script>

<span
  bind:this={anchor}
  role="presentation"
  class="contents"
  onpointerover={show}
  onpointerout={hide}
  onfocusin={show}
  onfocusout={hide}
>
  {@render children()}
</span>

{#if shown}
  <span
    bind:this={bubble}
    role="tooltip"
    class="pointer-events-none fixed z-50 rounded-md bg-surface px-2 py-1 text-xs whitespace-nowrap text-ink shadow-lg ring-1 ring-line"
    style="left: {at.x}px; top: {at.y}px"
  >
    {text}
  </span>
{/if}
