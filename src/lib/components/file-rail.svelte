<script lang="ts">
  import { FileText, Search, SearchX, X } from '@lucide/svelte';
  import PathTail from '$lib/components/path-tail.svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import { untrack, type Snippet } from 'svelte';
  import type { Row, Seeking, Tally } from '$lib/bindings';
  import { app } from '$lib/app.svelte';
  import { rail } from '$lib/rail.svelte';
  import FileMenu from '$lib/components/file-menu.svelte';
  import ScrollBar from '$lib/components/scroll-bar.svelte';
  import FindToggles from '$lib/components/find-toggles.svelte';
  import BlankState from '$lib/components/blank-state.svelte';
  import type { ScopeActions, Target } from '$lib/components/types';
  import { sumTallies } from '$lib/format';
  import { climb, shared, shortened } from '$lib/scope';
  import { PLAIN, pattern } from '$lib/seek';

  type Band = {
    id: 'chosen' | 'rest';
    title: string;
    rows: Row[];
    excluded: boolean;
  };

  type BandLine = {
    kind: 'band';
    id: string;
    top: number;
    tall: number;
    band: Band;
    tally: Tally;
    standing: boolean;
  };

  type RowLine = {
    kind: 'row';
    id: string;
    top: number;
    tall: number;
    row: Row;
  };

  type Line = BandLine | RowLine;

  let {
    listed,
    open,
    searching,
    filtering,
    matches,
    narrowed,
    nameFilter = $bindable(''),
    nameHow = $bindable(),
    railBottom,
    onopen,
    acts,
  }: {
    listed: Row[];
    open: string | null;
    searching: boolean;
    filtering: boolean;
    matches: Record<string, number>;
    narrowed: boolean;
    nameFilter?: string;
    nameHow: Seeking;
    railBottom?: Snippet<[boolean]>;
    onopen: (row: Row) => void;
    acts: ScopeActions;
  } = $props();

  let filterOpen = $state(false);
  let box = $state<HTMLInputElement | null>(null);
  let menu = $state<ReturnType<typeof FileMenu> | null>(null);
  let marked = $state<string | null>(null);
  const closed = new SvelteSet<string>();

  const SPARE = 8;

  let bandTall = $state(34);
  let rowTall = $state(38);

  let scrolled = $state(0);
  let viewTall = $state(0);

  function flip(id: string, standing: boolean) {
    if (lone || narrowing) return;

    if (standing) {
      closed.add(id);
    } else {
      closed.delete(id);
    }
  }

  let list = $state<HTMLElement | null>(null);

  const settling = $derived(rail.loading || searching);
  const narrowing = $derived(filtering || narrowed);

  const byScope = $derived(new Map(rail.rows.map((one) => [one.scope, one])));
  const head = $derived(shared(rail.rows.map((one) => one.label)));

  const busy = $derived.by(() => {
    const out: Record<string, true> = {};

    for (const key of app.busy) {
      const row = climb(key, (at) => byScope.get(at));
      if (row) out[row.scope] = true;
    }

    return out;
  });

  const sections = $derived.by(() => {
    const out: Band[] = [];
    const kept = listed.filter((one) => !one.excluded);
    const rest = listed.filter((one) => one.excluded);

    if (kept.length) {
      out.push({
        id: 'chosen',
        title: 'Translating',
        rows: kept,
        excluded: false,
      });
    }

    if (rest.length) {
      out.push({
        id: 'rest',
        title:
          !kept.length && !narrowing
            ? 'Pick what to translate'
            : 'Not translating',
        rows: rest,
        excluded: true,
      });
    }

    return out;
  });

  function bandOpen(band: Band) {
    return lone || narrowing || !closed.has(band.id);
  }

  const lone = $derived(sections.length === 1 && sections[0]?.id === 'chosen');
  const aiming = $derived(marked !== null);
  const broken = $derived(
    nameFilter.trim() !== '' && pattern(nameFilter, nameHow) === null,
  );

  function targetOver(
    these: Row[],
    at: string,
    excluded: boolean,
    busy: boolean,
  ) {
    const counted = sumTallies(these.map((one) => one.tally));

    return {
      at,
      scopes: these.map((one) => one.scope),
      excluded,
      applied: counted.applied > 0,
      translated: counted.translated,
      busy,
    };
  }

  function aim(event: MouseEvent, target: Target) {
    menu?.open(event, target);
    marked = target.at;
  }

  function openFilter() {
    filterOpen = true;
    queueMicrotask(() => box?.select());
  }

  function closeFilter() {
    filterOpen = false;
    nameFilter = '';
    nameHow = { ...PLAIN };
  }

  $effect(() => {
    if (!open) return;

    const band = sections.find((one) =>
      one.rows.some((row) => row.scope === open),
    );
    if (band) closed.delete(band.id);
  });

  const lines = $derived.by(() => {
    const out: Line[] = [];
    let top = 0;

    for (const band of sections) {
      const standing = bandOpen(band);

      if (!lone) {
        out.push({
          kind: 'band',
          id: band.id,
          top,
          tall: bandTall,
          band,
          standing,
          tally: sumTallies(band.rows.map((one) => one.tally)),
        });
        top += bandTall;
      }

      if (!standing) continue;

      for (const row of band.rows) {
        out.push({
          kind: 'row',
          id: `${band.id}/${row.scope}`,
          top,
          tall: rowTall,
          row,
        });
        top += rowTall;
      }
    }

    return out;
  });

  const listTall = $derived(
    lines.length
      ? lines[lines.length - 1].top + lines[lines.length - 1].tall
      : 0,
  );

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

    const from = Math.max(0, rowAt(scrolled) - SPARE);
    const to = Math.min(lines.length, rowAt(scrolled + viewTall) + SPARE + 1);

    return lines.slice(from, to);
  });

  let measured = false;

  $effect(() => {
    void listed.length;

    if (measured) return;

    const root = list;
    if (!root) return;

    const gauge = (kind: string) =>
      root.querySelector(`[data-tall='${kind}']`)?.getBoundingClientRect()
        .height ?? 0;

    const band = gauge('band');
    const row = gauge('row');
    if (!row) return;

    if (band && band !== bandTall) bandTall = band;
    if (row !== rowTall) rowTall = row;

    measured = true;
  });

  const stirring = (one: Line) =>
    one.kind === 'band'
      ? one.band.rows.some((row) => busy[row.scope])
      : (busy[one.row.scope] ?? false);

  function targetFor(one: Line): Target {
    return one.kind === 'band'
      ? targetOver(one.band.rows, one.band.id, one.band.excluded, stirring(one))
      : targetOver([one.row], one.row.scope, one.row.excluded, stirring(one));
  }

  function bring(scope: string) {
    const root = list;
    if (!root) return;

    const one = lines.find(
      (line) => line.kind === 'row' && line.row.scope === scope,
    );
    if (!one) return;

    if (one.top < root.scrollTop) root.scrollTop = one.top;
    else if (one.top + one.tall > root.scrollTop + root.clientHeight) {
      root.scrollTop = one.top + one.tall - root.clientHeight;
    }
  }

  $effect(() => {
    void listed;

    const scope = open;
    if (!scope) return;

    untrack(() => bring(scope));
  });
</script>

<aside class="flex min-h-0 flex-col overflow-hidden bg-surface">
  <div class="flex h-11 shrink-0 items-center gap-1 pr-3 pl-4.5">
    {#if filterOpen}
      <input
        bind:this={box}
        bind:value={nameFilter}
        onkeydown={(event) => {
          if (event.isComposing) return;
          if (event.key !== 'Escape') return;

          event.stopPropagation();
          closeFilter();
        }}
        placeholder="Search"
        class="bare-input"
      />
      {#if broken}
        <span class="shrink-0 text-xs text-alarm">bad pattern</span>
      {/if}
      <FindToggles bind:how={nameHow} onpick={() => box?.focus()} />
      <button
        class="icon-button size-7 shrink-0"
        aria-label="Close filter"
        onclick={closeFilter}
      >
        <X class="size-4" />
      </button>
    {:else}
      <span class="min-w-0 flex-1 truncate text-xs text-ink-soft tabular-nums">
        {#if settling}
          <span class="block h-2.5 w-24 animate-pulse rounded-full bg-sunken"
          ></span>
        {:else if filtering}
          {listed.length} file{listed.length === 1 ? '' : 's'} with matches
        {:else if rail.chosen.files === 0}
          {rail.whole.total.toLocaleString()} line{rail.whole.total === 1
            ? ''
            : 's'} found
        {:else if rail.chosen.total === 0}
          {rail.chosen.files} file{rail.chosen.files === 1 ? '' : 's'} chosen, no
          text in them
        {:else}
          {rail.chosen.translated.toLocaleString()} of {rail.chosen.total.toLocaleString()}
          lines translated
        {/if}
      </span>
      <button
        class="icon-button size-7 shrink-0"
        aria-label="Filter files by name, path or type"
        onclick={openFilter}
      >
        <Search class="size-4" />
      </button>
    {/if}
  </div>

  <div class="group/scroll relative min-h-0 flex-1">
    <ScrollBar
      of="file-rail-list"
      tall={listTall}
      view={viewTall}
      at={scrolled}
      onmove={(top) => list?.scrollTo({ top })}
    />

    <div
      bind:this={list}
      id="file-rail-list"
      bind:clientHeight={viewTall}
      onscroll={(event) => (scrolled = event.currentTarget.scrollTop)}
      class="h-full overflow-auto bg-surface px-2 pt-1 pb-2"
    >
      {#if settling}
        <ol class="flex flex-col gap-0.5">
          {#each Array(8) as _unused, index (index)}
            <li class="flex h-9 animate-pulse items-center px-2.5">
              <span
                class="block h-2.5 rounded-full bg-sunken"
                style="width: {55 + ((index * 13) % 40)}%"
              ></span>
            </li>
          {/each}
        </ol>
      {:else if lines.length}
        <ol class="relative" style="height: {listTall}px">
          {#each visible as one (one.id)}
            <li
              oncontextmenu={(event) => aim(event, targetFor(one))}
              data-tall={one.kind}
              class="absolute right-0 left-0 pb-0.5"
              style="top: {one.top}px"
            >
              {#if one.kind === 'band'}
                {@render bandLine(one)}
              {:else}
                {@render rowLine(one)}
              {/if}
            </li>
          {/each}
        </ol>
      {:else}
        <div class="flex h-full flex-col items-center justify-center gap-3">
          {#if filtering || nameFilter}
            <BlankState Icon={SearchX} said="No matching file" />
          {:else}
            <BlankState Icon={FileText} said="No files to translate" />
          {/if}
        </div>
      {/if}
    </div>
  </div>

  {#if railBottom}
    <div class="@container shrink-0 px-2 pt-2 pb-3">
      {@render railBottom(rail.loading)}
    </div>
  {/if}
</aside>

{#snippet bandLine(one: BandLine)}
  <div
    class="flex items-center rounded-md py-1 {marked === one.band.id
      ? 'bg-sunken'
      : ''}"
  >
    <button
      type="button"
      onclick={() => flip(one.band.id, one.standing)}
      class="flex min-w-0 flex-1 items-center gap-1.5 px-2.5 text-left"
    >
      <span
        class="min-w-0 flex-1 truncate text-[10px] font-medium tracking-wider uppercase {one.standing
          ? 'text-ink-soft'
          : 'text-ink-faint'}"
      >
        {one.band.title}
      </span>

      {@render tally(
        one.band.excluded,
        stirring(one),
        one.tally.translated,
        one.tally.total,
        null,
      )}
    </button>
  </div>
{/snippet}

{#snippet rowLine(one: RowLine)}
  {@const row = one.row}
  <div
    class="group flex items-center gap-1.5 rounded-md pl-2.5 {open === row.scope
      ? 'bg-selected'
      : marked === row.scope
        ? 'bg-sunken'
        : aiming
          ? ''
          : 'hover:bg-sunken'}"
  >
    <button
      type="button"
      onclick={() => onopen(row)}
      class="flex min-w-0 flex-1 items-baseline justify-between gap-3 py-2.5 pr-2.5 text-left"
    >
      <span
        class="min-w-0 font-mono text-xs {row.excluded
          ? 'text-ink-faint'
          : open === row.scope
            ? 'font-medium text-on-selected'
            : marked === row.scope
              ? 'text-ink'
              : aiming
                ? 'text-ink-soft'
                : 'text-ink-soft group-hover:text-ink'}"
      >
        <PathTail path={shortened(row.label, head)} />
      </span>

      {@render tally(
        row.excluded,
        stirring(one),
        row.tally.translated,
        row.tally.total,
        filtering ? (matches[row.scope] ?? 0) : null,
      )}
    </button>
  </div>
{/snippet}

{#snippet tally(
  excluded: boolean,
  busy: boolean,
  translated: number,
  total: number,
  found: number | null,
)}
  {@const finished =
    found === null && !excluded && total > 0 && translated === total}
  <span
    class="flex shrink-0 items-center gap-1.5 text-[11px] tabular-nums {excluded
      ? 'text-ink-faint'
      : finished
        ? 'text-done'
        : 'text-ink-soft'}"
  >
    {#if busy}
      <span
        class="size-2.5 animate-spin rounded-full border border-accent-wash border-t-accent"
      ></span>
    {/if}
    {#if found !== null}
      {found}
    {:else if total > 0}
      {#if excluded}{total}{:else}{translated}/{total}{/if}
    {/if}
  </span>
{/snippet}

<FileMenu bind:this={menu} onclose={() => (marked = null)} {acts} />
