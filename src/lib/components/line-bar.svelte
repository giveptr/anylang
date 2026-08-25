<script lang="ts">
  import { Check, ListFilter, Search, X } from '@lucide/svelte';
  import type { Counts, Only, Seeking, Show } from '$lib/bindings';
  import FindToggles from '$lib/components/find-toggles.svelte';
  import { KINDS, namedKind } from '$lib/wording';

  let {
    trail,
    counts,
    loading,
    refused,
    show,
    only,
    piled,
    filterOpen = $bindable(),
    filter = $bindable(),
    how = $bindable(),
    onshow,
    ononly,
    onclose,
    onreseek,
  }: {
    trail: { under: string; leaf: string } | null;
    counts: Counts;
    loading: boolean;
    refused: string;
    show: Show;
    only: Only;
    piled: boolean;
    filterOpen: boolean;
    filter: string;
    how: Seeking;
    onshow: (wanted: Show) => void;
    ononly: (wanted: Only) => void;
    onclose: () => void;
    onreseek: () => void;
  } = $props();

  let box = $state<HTMLInputElement | null>(null);
  let sieveOpen = $state(false);
  let sieveBox = $state<HTMLElement | null>(null);

  const TOOL =
    'flex h-6 shrink-0 items-center gap-1.5 rounded transition-colors hover:bg-sunken hover:text-ink';

  function openFilter() {
    filterOpen = true;
    queueMicrotask(() => box?.select());
  }

  function reseek() {
    box?.focus();
    onreseek();
  }

  function sieve(wanted: Only) {
    sieveOpen = false;
    ononly(wanted);
  }
</script>

<svelte:window
  onpointerdown={(event) => {
    if (!sieveOpen) return;
    if (!sieveBox?.contains(event.target as Node)) sieveOpen = false;
  }}
/>

<div
  class="relative z-20 flex h-11 shrink-0 items-center gap-3 bg-surface pr-3.5 pl-5"
>
  {#if trail && !filterOpen}
    <span class="flex min-w-0 flex-1 items-baseline font-mono text-xs">
      {#if trail.under}
        <span class="min-w-0 truncate text-ink-faint">{trail.under}</span>
        <span class="shrink-0 text-ink-faint">/</span>
      {/if}
      <span class="shrink-0 text-ink-soft">{trail.leaf}</span>
    </span>
  {/if}

  {#if filterOpen}
    <div class="flex min-w-0 flex-1 items-center gap-2.5">
      <input
        bind:this={box}
        bind:value={filter}
        onkeydown={(event) => {
          if (event.isComposing) return;
          if (event.key !== 'Escape') return;

          event.stopPropagation();
          onclose();
        }}
        placeholder="Search in {trail?.leaf ?? 'this file'}"
        class="bare-input"
      />
      {#if refused}
        <span class="max-w-64 shrink-0 truncate text-xs text-alarm">
          {refused}
        </span>
      {:else if filter}
        <span class="shrink-0 text-xs text-ink-soft tabular-nums"
          >{counts.total}</span
        >
      {/if}
      <FindToggles bind:how onpick={reseek} />
      <button
        class="icon-button size-7 shrink-0"
        aria-label="Close file search"
        onclick={onclose}
      >
        <X class="size-4" />
      </button>
    </div>
  {/if}

  {#if trail}
    <div class="flex shrink-0 items-center gap-2">
      <div class="flex gap-0.5 rounded-md bg-sunken p-0.5 text-xs" role="group">
        {#each ['all', 'translated', 'untranslated'] as const as option (option)}
          <button
            type="button"
            onclick={() => onshow(option)}
            class="rounded px-2.5 py-1 capitalize transition-colors {show ===
            option
              ? 'bg-selected text-on-selected'
              : 'text-ink-soft hover:text-ink'}"
          >
            {option}{#if !loading}<span class="ml-1 tabular-nums opacity-60"
                >{option === 'all' ? counts.total : counts[option]}</span
              >{/if}
          </button>
        {/each}
      </div>

      {#if piled}
        <div bind:this={sieveBox} class="relative shrink-0">
          <button
            type="button"
            onclick={() => (sieveOpen = !sieveOpen)}
            aria-expanded={sieveOpen}
            aria-label="Narrow down to a kind of line"
            class="{TOOL} {only === 'yours'
              ? 'w-6 justify-center'
              : 'px-2 text-xs'} {only !== 'yours' || sieveOpen
              ? 'bg-sunken text-ink'
              : 'text-ink-faint'}"
          >
            <ListFilter class="size-4 shrink-0" />
            {#if only !== 'yours'}<span>{namedKind(only)}</span>{/if}
          </button>

          {#if sieveOpen}
            <div
              class="absolute top-full right-0 z-40 mt-1.5 w-44 overflow-hidden rounded-lg bg-surface shadow-xl ring-1 ring-line"
            >
              {#each KINDS as [option, said] (option)}
                <button
                  type="button"
                  onclick={() => sieve(option)}
                  class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs transition-colors {only ===
                  option
                    ? 'bg-sunken text-ink'
                    : 'text-ink-soft hover:bg-sunken hover:text-ink'}"
                >
                  <Check
                    class="size-3.5 shrink-0 {only === option
                      ? ''
                      : 'opacity-0'}"
                  />
                  {said}
                </button>
              {/each}
            </div>
          {/if}
        </div>
      {/if}

      {#if !filterOpen}
        <button
          class="{TOOL} w-6 justify-center text-ink-faint"
          aria-label="Search in this file"
          onclick={openFilter}
        >
          <Search class="size-4" />
        </button>
      {/if}
    </div>
  {/if}
</div>
