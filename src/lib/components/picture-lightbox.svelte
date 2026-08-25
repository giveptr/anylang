<script lang="ts" module>
  let raised = $state(0);

  export const veiled = () => raised > 0;
</script>

<script lang="ts">
  import { onMount, type Snippet } from 'svelte';
  import { X } from '@lucide/svelte';

  type Props = {
    src: string;
    name: string;
    why?: string;
    onclose: () => void;
    onkey?: (event: KeyboardEvent) => void;
    actions?: Snippet;
  };

  let { src, name, why = '', onclose, onkey, actions }: Props = $props();

  const NEAREST = 32;
  const BUDGE = 4;

  let box = $state<HTMLElement | null>(null);
  let pic = $state<HTMLImageElement | null>(null);
  let zoom = $state(1);
  let slid = $state({ x: 0, y: 0 });
  let fit = $state(0);
  let grip = $state<{
    x: number;
    y: number;
    onPic: boolean;
    crept: number;
    heldX: number;
    heldY: number;
  } | null>(null);
  let queued = 0;

  const grown = $derived(zoom * fit);
  const sharp = $derived(grown >= 2);
  const percent = $derived(Math.round(grown * 100));

  onMount(() => {
    raised += 1;
    return () => {
      raised -= 1;
      if (queued) cancelAnimationFrame(queued);
    };
  });

  function measure() {
    if (pic?.naturalWidth) fit = pic.clientWidth / pic.naturalWidth;
  }

  function home() {
    zoom = 1;
    slid = { x: 0, y: 0 };
  }

  function middle() {
    const seen = box?.getBoundingClientRect();
    if (!seen) return { x: window.innerWidth / 2, y: window.innerHeight / 2 };

    return { x: seen.left + seen.width / 2, y: seen.top + seen.height / 2 };
  }

  function toward(next: number, atX: number, atY: number) {
    if (next === 1) {
      home();
      return;
    }

    const mid = middle();
    const cx = atX - mid.x;
    const cy = atY - mid.y;
    const grew = next / zoom;

    slid = { x: cx - grew * (cx - slid.x), y: cy - grew * (cy - slid.y) };
    zoom = next;
  }

  function slide(value: number) {
    const next = 2 ** value;
    if (next === 1) {
      home();
      return;
    }

    const grew = next / zoom;

    slid = { x: grew * slid.x, y: grew * slid.y };
    zoom = next;
  }

  function onwheel(event: WheelEvent) {
    event.preventDefault();

    const next = Math.min(
      NEAREST,
      Math.max(1, zoom * Math.exp(-event.deltaY / 600)),
    );
    if (next !== zoom) toward(next, event.clientX, event.clientY);
  }

  function grab(event: PointerEvent) {
    if (event.button !== 0) return;

    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    grip = {
      x: event.clientX,
      y: event.clientY,
      onPic: event.target === pic,
      crept: 0,
      heldX: 0,
      heldY: 0,
    };
  }

  function drag(event: PointerEvent) {
    if (!grip) return;

    const dx = event.clientX - grip.x;
    const dy = event.clientY - grip.y;

    grip.x = event.clientX;
    grip.y = event.clientY;
    grip.crept += Math.abs(dx) + Math.abs(dy);
    grip.heldX += dx;
    grip.heldY += dy;

    if (queued) return;
    queued = requestAnimationFrame(() => {
      queued = 0;
      settle();
    });
  }

  function settle() {
    if (!grip) return;

    slid = { x: slid.x + grip.heldX, y: slid.y + grip.heldY };
    grip.heldX = 0;
    grip.heldY = 0;
  }

  function dropped() {
    if (queued) {
      cancelAnimationFrame(queued);
      queued = 0;
    }

    grip = null;
  }

  function loose() {
    if (!grip) return;

    settle();
    const tapped = grip.crept <= BUDGE && !grip.onPic;
    dropped();
    if (tapped) onclose();
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      onclose();
      return;
    }

    if (!event.defaultPrevented && !(event.target instanceof HTMLInputElement))
      onkey?.(event);
  }

  $effect(() => {
    void src;
    home();
    fit = 0;
  });
</script>

<svelte:window {onkeydown} onresize={measure} />

<div
  role="dialog"
  aria-modal="true"
  aria-label={name}
  tabindex="-1"
  data-theme="dark"
  bind:this={box}
  class="absolute inset-0 z-50 touch-none overflow-hidden bg-ground/90 text-ink select-none"
  {onwheel}
  ondblclick={home}
  onpointerdown={grab}
  onpointermove={drag}
  onpointerup={loose}
  onpointercancel={dropped}
>
  <div class="absolute inset-0 flex items-center justify-center p-10">
    {#if src}
      <img
        bind:this={pic}
        {src}
        alt={name}
        draggable="false"
        onload={measure}
        class="max-h-full max-w-full object-contain will-change-transform {grip
          ? 'cursor-grabbing'
          : 'cursor-grab'}"
        style="transform: translate({slid.x}px, {slid.y}px) scale({zoom});{sharp
          ? ' image-rendering: pixelated;'
          : ''}"
      />
    {:else if why}
      <p class="max-w-sm text-center text-sm text-ink-soft">{why}</p>
    {:else}
      <span
        class="size-6 animate-spin rounded-full border-2 border-sunken border-t-ink-soft"
      ></span>
    {/if}
  </div>

  <div
    role="toolbar"
    tabindex="-1"
    class="absolute top-3 left-1/2 flex -translate-x-1/2 items-center gap-2 rounded-full bg-surface/95 py-1 pr-1 pl-3 shadow-lg ring-1 ring-line"
    onpointerdown={(event) => event.stopPropagation()}
    ondblclick={(event) => event.stopPropagation()}
  >
    <span class="max-w-72 truncate text-xs">{name}</span>
    <input
      class="h-1 w-24 cursor-pointer appearance-none rounded-full bg-edge [&::-webkit-slider-thumb]:size-3 [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-ink"
      type="range"
      aria-label="Zoom"
      min="0"
      max={Math.log2(NEAREST)}
      step="0.01"
      value={Math.log2(zoom)}
      oninput={(event) => slide(event.currentTarget.valueAsNumber)}
    />
    {#if fit > 0}
      <span class="min-w-12 text-right text-[11px] text-ink-soft tabular-nums">
        {percent}%
      </span>
    {/if}
    <button
      class="grid size-6 place-items-center rounded-full text-ink-soft transition-colors hover:bg-sunken hover:text-ink"
      aria-label="Close this view"
      onclick={onclose}
    >
      <X class="size-3.5" />
    </button>
  </div>

  {#if actions}
    <div
      role="toolbar"
      tabindex="-1"
      class="absolute right-3 bottom-3 flex items-center gap-1 rounded-full bg-surface/95 p-1 shadow-lg ring-1 ring-line"
      onpointerdown={(event) => event.stopPropagation()}
      ondblclick={(event) => event.stopPropagation()}
    >
      {@render actions()}
    </div>
  {/if}
</div>
