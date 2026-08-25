<script lang="ts">
  import { ChevronDown, ChevronUp, Search, X } from '@lucide/svelte';
  import type { Seeking } from '$lib/bindings';
  import { PLAIN, longEnough, pattern } from '$lib/seek';
  import { app } from '$lib/app.svelte';
  import FindToggles from '$lib/components/find-toggles.svelte';

  let {
    query = $bindable(),
    how = $bindable(),
    open = $bindable(),
    total,
    files,
    at,
    onstep,
    searching,
  }: {
    query: string;
    how: Seeking;
    open: boolean;
    total: number;
    files: number;
    at: number;
    onstep: (by: number) => void;
    searching: boolean;
  } = $props();

  let box = $state<HTMLInputElement | null>(null);

  const ready = $derived(longEnough(query.trim()));
  const broken = $derived(ready && pattern(query, how) === null);

  function onkeydown(event: KeyboardEvent) {
    if (app.view !== 'text') return;
    if (!(event.ctrlKey || event.metaKey) || event.key !== 'f') return;

    event.preventDefault();
    open = true;
    queueMicrotask(() => box?.select());
  }

  function onboxkey(event: KeyboardEvent) {
    if (event.isComposing) return;
    if (event.key === 'Escape') {
      event.stopPropagation();
      close();
      return;
    }

    if (event.key !== 'Enter') return;

    event.preventDefault();
    onstep(event.shiftKey ? -1 : 1);
  }

  function close() {
    open = false;
    query = '';
    how = { ...PLAIN };
  }
</script>

<svelte:window {onkeydown} />

{#if open}
  <div
    class="flex h-11 shrink-0 items-center gap-2.5 border-b border-line bg-surface pr-3.5 pl-4.5"
  >
    <Search class="size-4 shrink-0 text-ink-faint" />

    <input
      bind:this={box}
      bind:value={query}
      onkeydown={onboxkey}
      placeholder="Search everything (Ctrl F)"
      class="bare-input"
    />

    <FindToggles bind:how onpick={() => box?.focus()} />

    <div class="h-5 w-px shrink-0 bg-line"></div>

    {#if broken}
      <span class="shrink-0 text-xs text-alarm">bad pattern</span>
    {:else if ready && !searching}
      <span class="shrink-0 text-xs text-ink-soft tabular-nums">
        {#if total === 0}
          no matches
        {:else if at >= 0}
          {at + 1}/{total}
        {:else}
          {total} in {files} file{files === 1 ? '' : 's'}
        {/if}
      </span>
    {/if}

    {#if ready && !broken && !searching && total > 0}
      <button
        class="icon-button size-7 shrink-0"
        aria-label="Previous match"
        onmousedown={(event) => event.preventDefault()}
        onclick={() => {
          onstep(-1);
          box?.focus();
        }}
      >
        <ChevronUp class="size-4" />
      </button>
      <button
        class="icon-button size-7 shrink-0"
        aria-label="Next match"
        onmousedown={(event) => event.preventDefault()}
        onclick={() => {
          onstep(1);
          box?.focus();
        }}
      >
        <ChevronDown class="size-4" />
      </button>
    {/if}

    <button
      class="icon-button size-7 shrink-0"
      aria-label="Close search"
      onclick={close}
    >
      <X class="size-4" />
    </button>
  </div>
{/if}
