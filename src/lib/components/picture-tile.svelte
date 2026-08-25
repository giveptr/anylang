<script lang="ts">
  import { ImageOff, Lock, Star } from '@lucide/svelte';
  import type { Shot } from '$lib/bindings';
  import {
    TILE,
    marked,
    shown,
    swappedTo,
    whyBlank,
  } from '$lib/pictures.svelte';

  type Props = {
    shot: Shot;
    chosen: boolean;
    over: boolean;
    onopen: (shot: Shot) => void;
    onmenu: (event: MouseEvent, shot: Shot) => void;
  };

  let { shot, chosen, over, onopen, onmenu }: Props = $props();

  const source = $derived(shot.drawable ? shown(shot.key, TILE) : '');
  const replaced = $derived(!!swappedTo(shot.key));
  const blank = $derived(!shot.drawable || !!whyBlank(shot.key, TILE));
</script>

<button
  data-key={shot.key}
  class="group flex h-full min-h-0 flex-col gap-1.5 rounded-lg p-2 text-left transition-colors {over
    ? 'bg-accent-wash inset-ring-2 inset-ring-accent'
    : chosen
      ? 'bg-selected inset-ring-2 inset-ring-accent'
      : 'hover:bg-sunken'}"
  onclick={() => onopen(shot)}
  oncontextmenu={(event) => onmenu(event, shot)}
>
  <div
    class="relative flex min-h-0 w-full flex-1 items-center justify-center overflow-hidden rounded-md checkers ring-1 ring-edge"
  >
    {#if marked(shot.key)}
      <span
        class="absolute top-1 right-1 grid place-items-center rounded-full bg-surface/90 p-1 shadow-sm ring-1 ring-line"
      >
        <Star class="size-3 fill-pending text-pending" />
      </span>
    {/if}
    {#if source}
      <img
        src={source}
        alt={shot.name}
        draggable="false"
        class="max-h-full max-w-full object-contain"
        style="image-rendering: {shot.wide < 64 ? 'pixelated' : 'auto'}"
      />
    {:else if blank}
      <ImageOff class="size-5 text-ink-faint" />
    {/if}
  </div>

  <div class="flex min-w-0 items-center gap-1">
    {#if replaced}
      <span class="size-1.5 shrink-0 rounded-full bg-accent"></span>
    {/if}
    {#if shot.locked}
      <Lock class="size-3 shrink-0 text-ink-faint" />
    {/if}
    <span
      class="min-w-0 flex-1 truncate text-xs {chosen
        ? 'font-medium text-on-selected'
        : ''}"
    >
      {shot.name}
    </span>
  </div>

  <span class="truncate text-[11px] text-ink-faint tabular-nums">
    {shot.wide}×{shot.high}
    {#if replaced}
      · replaced
    {/if}
  </span>
</button>
