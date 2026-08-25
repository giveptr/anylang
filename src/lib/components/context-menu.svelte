<script lang="ts">
  import { setContext, type Snippet } from 'svelte';

  let {
    children,
    onclose,
  }: {
    children: Snippet;
    onclose?: () => void;
  } = $props();

  setContext('menu', { close: () => close() });

  const EDGE = 6;
  const GAP = 4;

  let spot = $state<{ x: number; y: number; centred: boolean } | null>(null);
  let place = $state({ left: 0, top: 0 });
  let panel = $state<HTMLDivElement | null>(null);
  let anchor: HTMLElement | null = null;

  export const shown = () => spot !== null;

  export function open(event: MouseEvent) {
    event.preventDefault();
    close();
    spot = { x: event.clientX, y: event.clientY, centred: false };
    place = { left: event.clientX, top: event.clientY };
  }

  export function openBelow(from: HTMLElement) {
    if (spot) {
      close();
      return;
    }

    const box = from.getBoundingClientRect();
    anchor = from;
    spot = { x: box.left + box.width / 2, y: box.bottom + GAP, centred: true };
    place = { left: box.left, top: box.bottom + GAP };
  }

  export function close() {
    if (!spot) return;

    spot = null;
    anchor = null;
    onclose?.();
  }

  $effect(() => {
    if (!spot || !panel) return;

    const box = panel.getBoundingClientRect();
    const left = spot.centred ? spot.x - box.width / 2 : spot.x;

    place = {
      left: Math.max(
        EDGE,
        Math.min(left, window.innerWidth - box.width - EDGE),
      ),
      top: Math.max(
        EDGE,
        Math.min(spot.y, window.innerHeight - box.height - EDGE),
      ),
    };
  });

  function onpointerdown(event: PointerEvent) {
    if (!spot) return;

    const target = event.target as Node;
    if (panel?.contains(target) || anchor?.contains(target)) return;

    close();
  }

  function oncontextmenu(event: MouseEvent) {
    if (!spot || event.defaultPrevented) return;
    if (panel?.contains(event.target as Node)) return;

    close();
  }

  function onkeydown(event: KeyboardEvent) {
    if (spot && event.key === 'Escape') close();
  }
</script>

<svelte:window
  {onpointerdown}
  {oncontextmenu}
  {onkeydown}
  onwheel={close}
  onresize={close}
  onblur={close}
/>

{#if spot}
  <div
    bind:this={panel}
    class="fixed z-50 min-w-40 overflow-hidden rounded-lg bg-surface shadow-xl ring-1 ring-line"
    style="left: {place.left}px; top: {place.top}px"
  >
    {@render children()}
  </div>
{/if}
