<script lang="ts">
  import {
    ChevronDown,
    ChevronRight,
    Images,
    ImageUp,
    Search,
    SearchX,
    Star,
    X,
  } from '@lucide/svelte';
  import { tick, untrack } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import { type Branch, chainTo, sifted } from '$lib/branches';
  import type { Shot } from '$lib/bindings';
  import { sheetsIn } from '$lib/shots';
  import type { Kept, Narrow } from '$lib/components/types';
  import PathTail from '$lib/components/path-tail.svelte';
  import ScrollBar from '$lib/components/scroll-bar.svelte';
  import BlankState from '$lib/components/blank-state.svelte';

  type Props = {
    shelves: Branch<Shot>[];
    count: number;
    replaced: number;
    marked: number;
    loading: boolean;
    openedIn: string;
    sift: Narrow;
    onnarrow: (onto: Partial<Narrow>) => void;
    onlook: (spot: string, atlas: string) => void;
  };

  let {
    shelves,
    count,
    replaced,
    marked,
    loading,
    openedIn,
    sift,
    onnarrow,
    onlook,
  }: Props = $props();

  type Shelf = {
    kind: 'shelf';
    one: Branch<Shot>;
    bare: boolean;
    branching: boolean;
  };

  type Sheet = { kind: 'sheet'; at: string; atlas: string; many: number };

  type Line = {
    id: string;
    top: number;
    tall: number;
    deep: number;
    row: Shelf | Sheet;
  };

  const SHELF_TALL = 36;
  const SHEET_TALL = 32;
  const AROUND = 8;
  const ROOM = SHELF_TALL * 3;
  const SPARE = 8;
  const NOTHING: ReadonlySet<string> = new Set();

  let box = $state<HTMLDivElement | null>(null);
  let scrolled = $state(0);
  let viewTall = $state(0);
  const open = new SvelteSet<string>();
  const shut = new SvelteSet<string>();
  let known: Branch<Shot>[] | null = null;

  let query = $state('');

  const everything = $derived(!sift.spot && sift.kept === 'every');

  type Filter = {
    kept: Kept;
    label: string;
    Icon: typeof Images;
    many: number;
  };

  const filters: Filter[] = $derived([
    { kept: 'replaced', label: 'Replaced', Icon: ImageUp, many: replaced },
    { kept: 'marked', label: 'Marked', Icon: Star, many: marked },
  ]);

  const ROW =
    'flex w-full items-center gap-1.5 rounded-md px-2.5 py-2.5 text-left text-xs transition-colors';
  const ROW_QUIET = 'text-ink-soft hover:bg-sunken hover:text-ink';

  const needle = $derived(query.trim().toLowerCase());
  const seeking = $derived(needle.length > 0);

  const found = $derived(
    seeking ? sifted(shelves, needle, (one) => one.atlas) : null,
  );

  const shown = $derived(found?.shown ?? shelves);
  const holds = $derived(found?.holds ?? NOTHING);

  const bared = (one: Branch<Shot>) =>
    !shut.has(one.at) && (open.has(one.at) || holds.has(one.at));

  function unfold(at: string) {
    shut.delete(at);
    open.add(at);
  }

  function fold(one: Branch<Shot>) {
    if (!bared(one)) {
      unfold(one.at);
      return;
    }

    open.delete(one.at);
    shut.add(one.at);
  }

  function into(one: Branch<Shot>) {
    unfold(one.at);
    onlook(one.at, '');
  }

  const picked = $derived(openedIn || sift.spot);
  const chain = $derived(picked ? chainTo(shelves, picked) : []);
  const here = $derived(chain.at(-1)?.at ?? '');

  const lines = $derived.by(() => {
    const out: Line[] = [];
    let top = 0;

    const add = (
      id: string,
      deep: number,
      tall: number,
      row: Shelf | Sheet,
    ) => {
      out.push({ id, top, tall, deep, row });
      top += tall;
    };

    const walk = (these: Branch<Shot>[], deep: number) => {
      for (const one of these) {
        const bare = bared(one);

        add(one.at, deep, SHELF_TALL, {
          kind: 'shelf',
          one,
          bare,
          branching: one.kids.length > 0 || one.own.some((held) => held.atlas),
        });

        if (!bare) continue;

        walk(one.kids, deep + 1);

        for (const [atlas, many] of sheetsIn(one.own)) {
          add(`${one.at}\u0000${atlas}`, deep, SHEET_TALL, {
            kind: 'sheet',
            at: one.at,
            atlas,
            many,
          });
        }
      }
    };

    walk(shown, 0);

    return out;
  });

  const listTall = $derived.by(() => {
    const last = lines.at(-1);

    return last ? last.top + last.tall : 0;
  });

  function rowAt(y: number) {
    let low = 0;
    let high = lines.length - 1;
    let best = 0;

    while (low <= high) {
      const mid = (low + high) >> 1;

      if (lines[mid].top <= y) {
        best = mid;
        low = mid + 1;
      } else {
        high = mid - 1;
      }
    }

    return best;
  }

  const visible = $derived.by(() => {
    if (!lines.length) return [];

    const y = scrolled - AROUND / 2;
    const from = Math.max(0, rowAt(y) - SPARE);
    const to = Math.min(lines.length, rowAt(y + viewTall) + SPARE + 1);

    return lines.slice(from, to);
  });

  function topFor(at: string) {
    const one = lines.find(
      (line) => line.row.kind === 'shelf' && line.id === at,
    );
    if (!one) return null;

    const top = AROUND / 2 + one.top;
    const least = top + one.tall + ROOM - viewTall;
    const most = top - ROOM;

    if (scrolled >= least && scrolled <= most) return null;

    const end = Math.max(0, listTall + AROUND - viewTall);

    return Math.min(Math.max(scrolled > most ? most : least, 0), end);
  }

  $effect(() => {
    const these = shelves;
    const path = chain;
    const row = here;

    untrack(() => {
      if (known !== these) {
        known = these;
        open.clear();
        shut.clear();
        scrolled = 0;
        box?.scrollTo({ top: 0 });
      }

      if (!row) return;

      for (const one of path) unfold(one.at);

      const top = topFor(row);
      if (top === null) return;

      void tick().then(() => box?.scrollTo({ top }));
    });
  });
</script>

<nav class="flex min-h-0 flex-col bg-surface">
  <div class="shrink-0 border-b border-line px-2 py-1">
    <button
      class="{ROW} {everything ? 'bg-selected' : ROW_QUIET}"
      onclick={() => onnarrow({})}
    >
      <Images class="size-3.5 shrink-0" />
      <span
        class="min-w-0 flex-1 truncate {everything
          ? 'font-medium text-on-selected'
          : ''}"
      >
        Every picture
      </span>
      {#if count > 0}
        <span class="shrink-0 text-xs text-ink-faint tabular-nums">
          {count}
        </span>
      {/if}
    </button>

    {#each filters as one (one.kept)}
      {@const on = sift.kept === one.kept}
      <button
        class="mt-0.5 {ROW} {on ? 'bg-selected' : ROW_QUIET}"
        onclick={() => onnarrow({ kept: one.kept })}
      >
        <one.Icon class="size-3.5 shrink-0" />
        <span
          class="min-w-0 flex-1 truncate {on
            ? 'font-medium text-on-selected'
            : ''}"
        >
          {one.label}
        </span>
        {#if one.many > 0}
          <span class="shrink-0 text-xs text-ink-faint tabular-nums">
            {one.many}
          </span>
        {/if}
      </button>
    {/each}
  </div>

  <div
    class="flex shrink-0 items-center gap-2 border-b border-line py-2 pr-3 pl-4.5"
  >
    <Search class="size-3.5 shrink-0 text-ink-faint" />
    <input
      bind:value={query}
      onkeydown={(event) => {
        if (event.isComposing || event.key !== 'Escape') return;

        event.stopPropagation();
        query = '';
      }}
      placeholder="Search"
      class="bare-input"
    />
    {#if query}
      <button
        class="grid size-5 shrink-0 place-items-center rounded text-ink-faint hover:bg-sunken hover:text-ink"
        aria-label="Clear the search"
        onclick={() => (query = '')}
      >
        <X class="size-3" />
      </button>
    {/if}
  </div>

  <div class="group/scroll relative min-h-0 flex-1">
    <ScrollBar
      of="shelf-rail"
      tall={listTall + AROUND}
      view={viewTall}
      at={scrolled}
      onmove={(top) => box?.scrollTo({ top })}
    />

    <div
      bind:this={box}
      id="shelf-rail"
      bind:clientHeight={viewTall}
      onscroll={(event) => (scrolled = event.currentTarget.scrollTop)}
      class="h-full overflow-auto px-2"
    >
      {#if loading}
        <div style="padding-block: {AROUND / 2}px">
          {#each Array(9) as _unused, index (index)}
            <div
              class="flex animate-pulse items-center px-2.5"
              style="height: {SHELF_TALL}px"
            >
              <span
                class="block h-2.5 rounded-full bg-sunken"
                style="width: {48 + ((index * 17) % 45)}%"
              ></span>
            </div>
          {/each}
        </div>
      {:else if lines.length}
        <div class="relative" style="height: {listTall + AROUND}px">
          {#each visible as line (line.id)}
            <div
              class="absolute right-0 left-0"
              style="top: {AROUND / 2 + line.top}px; height: {line.tall}px"
            >
              {#if line.row.kind === 'shelf'}
                {@render shelved(line.row, line.deep)}
              {:else}
                {@render sheeted(line.row, line.deep)}
              {/if}
            </div>
          {/each}
        </div>
      {:else if seeking}
        <div class="flex flex-col items-center gap-3 py-10">
          <BlankState Icon={SearchX} said="No matches" />
        </div>
      {/if}
    </div>
  </div>
</nav>

{#snippet shelved(row: Shelf, deep: number)}
  {@const one = row.one}
  {@const chosen = one.at === here && !sift.atlas}
  <div
    class="group flex h-full items-stretch rounded-md {chosen
      ? 'bg-selected'
      : 'hover:bg-sunken'}"
  >
    {#if deep > 0}
      <span class="shrink-0" style="width: {deep * 0.75}rem"></span>
    {/if}

    {#if row.branching}
      <button
        class="grid w-5 shrink-0 place-items-center text-ink-faint hover:text-ink"
        aria-label="{row.bare ? 'Hide' : 'Show'} what {one.label} holds"
        onclick={() => fold(one)}
      >
        {#if row.bare}
          <ChevronDown class="size-3" />
        {:else}
          <ChevronRight class="size-3" />
        {/if}
      </button>
    {/if}

    <button
      class="flex min-w-0 flex-1 items-center gap-1.5 pr-2.5 text-left text-xs {row.branching
        ? 'pl-0.5'
        : 'pl-2.5'} {chosen ? '' : 'text-ink-soft group-hover:text-ink'}"
      onclick={() => into(one)}
    >
      <span
        class="min-w-0 flex-1 font-mono text-xs {chosen
          ? 'font-medium text-on-selected'
          : ''}"
      >
        <PathTail path={one.label} />
      </span>
      <span class="shrink-0 text-xs text-ink-faint tabular-nums">
        {one.many}
      </span>
    </button>
  </div>
{/snippet}

{#snippet sheeted(row: Sheet, deep: number)}
  {@const inner = sift.spot === row.at && sift.atlas === row.atlas}
  <button
    class="flex h-full w-full items-center gap-1.5 rounded-md pr-2.5 pl-2.5 text-left text-xs {inner
      ? 'bg-selected'
      : 'text-ink-soft hover:bg-sunken hover:text-ink'}"
    onclick={() => onlook(row.at, row.atlas)}
  >
    <span class="shrink-0" style="width: {deep * 0.75 + 1.25}rem"></span>
    <span
      class="min-w-0 flex-1 truncate text-xs {inner
        ? 'font-medium text-on-selected'
        : ''}">{row.atlas}</span
    >
    <span class="shrink-0 text-[11px] text-ink-faint tabular-nums">
      {row.many}
    </span>
  </button>
{/snippet}
