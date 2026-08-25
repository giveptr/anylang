<script lang="ts">
  import { Search } from '@lucide/svelte';
  import type { Snippet } from 'svelte';
  import ScrollBar from '$lib/components/scroll-bar.svelte';

  let {
    open = $bindable(false),
    needle = $bindable(''),
    searchable = true,
    placeholder = 'Search',
    layout = 'items-center gap-2 px-3 py-2',
    onenter,
    trigger,
    children,
  }: {
    open?: boolean;
    needle?: string;
    searchable?: boolean;
    placeholder?: string;
    layout?: string;
    onenter?: () => void;
    trigger: Snippet;
    children: Snippet;
  } = $props();

  let box = $state<HTMLInputElement | null>(null);
  let panel = $state<HTMLDivElement | null>(null);
  let list = $state<HTMLElement | null>(null);

  let tall = $state(0);
  let view = $state(0);
  let scrolled = $state(0);

  const listed = $props.id();

  export function focus() {
    box?.focus();
  }

  $effect(() => {
    void open;
    void needle;

    const node = list;
    if (!node) return;

    const measure = requestAnimationFrame(() => (tall = node.scrollHeight));
    return () => cancelAnimationFrame(measure);
  });

  function toggle() {
    if (open) {
      open = false;
      return;
    }

    open = true;
    needle = '';
    if (searchable) queueMicrotask(() => box?.focus());
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.isComposing || event.key !== 'Enter') return;

    event.preventDefault();
    onenter?.();
  }

  function onpointerdown(event: PointerEvent) {
    if (!open || !panel) return;
    if (!panel.contains(event.target as Node)) open = false;
  }

  function onescape(event: KeyboardEvent) {
    if (!open || event.isComposing || event.key !== 'Escape') return;

    event.preventDefault();
    open = false;
  }
</script>

<svelte:window {onpointerdown} onkeydown={onescape} />

<div class="relative" bind:this={panel}>
  <button
    type="button"
    onclick={toggle}
    class="flex w-full rounded-lg bg-surface text-left text-sm ring-1 transition-colors {layout} {open
      ? 'ring-2 ring-accent'
      : 'ring-edge hover:ring-ink-faint'}"
  >
    {@render trigger()}
  </button>

  {#if open}
    <div
      class="absolute inset-x-0 top-full z-40 mt-1 flex max-h-64 flex-col overflow-hidden rounded-lg bg-surface shadow-xl ring-1 ring-line"
    >
      {#if searchable}
        <div class="flex shrink-0 items-center gap-2 px-3 py-2">
          <Search class="size-3.5 shrink-0 text-ink-faint" />
          <input
            bind:this={box}
            bind:value={needle}
            {onkeydown}
            {placeholder}
            class="bare-input"
          />
        </div>
      {/if}

      <div class="group/scroll relative flex min-h-0 flex-1 flex-col">
        <ScrollBar
          of={listed}
          {tall}
          {view}
          at={scrolled}
          onmove={(top) => list?.scrollTo({ top })}
        />

        <ol
          bind:this={list}
          id={listed}
          bind:clientHeight={view}
          onscroll={(event) => {
            scrolled = event.currentTarget.scrollTop;
            tall = event.currentTarget.scrollHeight;
          }}
          class="min-h-0 flex-1 overflow-auto bg-surface"
        >
          {@render children()}
        </ol>
      </div>
    </div>
  {/if}
</div>
