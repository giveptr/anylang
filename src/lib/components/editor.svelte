<script lang="ts">
  import {
    Check,
    FileText,
    Languages,
    ListFilter,
    MousePointerClick,
    SearchX,
  } from '@lucide/svelte';
  import {
    commands,
    type Entry,
    type Found,
    type Row,
    type Only,
    type Seeking,
    type Show,
    type Sift,
    type Window as Loaded,
  } from '$lib/bindings';
  import { alarm, app } from '$lib/app.svelte';
  import { NO_ASKED, NO_LISTED } from '$lib/wording';
  import { PLAIN, holds, pattern, texts } from '$lib/seek';
  import { watch } from '$lib/watch';
  import { within } from '$lib/scope';
  import { caught, type Outcome } from '$lib/save';
  import { credited, edited, loadRows, rail, rowOf } from '$lib/rail.svelte';
  import { scopeActions } from '$lib/scope-actions';
  import EntryRow from '$lib/components/entry-row.svelte';
  import { lineId, lineName } from '$lib/components/types';
  import LineBar from '$lib/components/line-bar.svelte';
  import ScopeBar from '$lib/components/scope-bar.svelte';
  import ScrollBar from '$lib/components/scroll-bar.svelte';
  import FileRail from '$lib/components/file-rail.svelte';
  import BlankState from '$lib/components/blank-state.svelte';

  import { onDestroy, tick, untrack, type Snippet } from 'svelte';

  let {
    gameDir,
    found,
    query,
    how,
    filtering,
    searching,
    spot,
    railBottom,
    onorder,
    onfile,
  }: {
    gameDir: string;
    found: Found[];
    query: string;
    how: Seeking;
    filtering: boolean;
    searching: boolean;
    spot: { key: string; id: number } | null;
    railBottom?: Snippet<[boolean]>;
    onorder: (scopes: string[]) => void;
    onfile: (key: string) => { key: string; id: number } | null;
  } = $props();

  type Viewing = {
    scope: string;
    name: string;
    under: string;
    files: number;
  };

  let opened = $state<Viewing | null>(null);
  const many = $derived((opened?.files ?? 0) > 1);
  let loaded = $state<Loaded | null>(null);
  let nameFilter = $state('');
  let byName = $state<RegExp | null>(null);
  let nameHow = $state<Seeking>({ ...PLAIN });
  let lineFilter = $state('');
  let asked = $state('');
  let lineHow = $state<Seeking>({ ...PLAIN });
  let lineFilterOpen = $state(false);
  let box = $state<HTMLElement | null>(null);
  let show = $state<Show>('all');
  let only = $state<Only>('yours');
  let loadingLines = $state(false);
  let editing = $state<string | null>(null);
  let scrolled = $state(0);
  let viewTall = $state(0);
  let listTall = $state(0);

  const isDone = (entry: Entry) => entry.translation !== null;

  const matches = $derived.by(() => {
    const out: Record<string, number> = {};

    for (const one of found) {
      const row = rowOf(one.key);
      if (row) out[row.scope] = (out[row.scope] ?? 0) + one.lines.length;
    }

    return out;
  });

  const listed = $derived.by(() => {
    let out = rail.rows;
    if (filtering) out = out.filter((one) => matches[one.scope]);

    const mark = byName;
    if (mark)
      out = out.filter((one) => texts(one).some((text) => holds(mark, text)));

    return out;
  });

  $effect(() => {
    const built = pattern(nameFilter, nameHow);
    if (!built) {
      byName = null;
      return;
    }

    const timer = setTimeout(() => (byName = built), 120);
    return () => clearTimeout(timer);
  });

  const lines = $derived(loaded?.lines ?? []);
  const counts = $derived(
    loaded?.counts ?? { total: 0, translated: 0, untranslated: 0 },
  );

  const loading = $derived(rail.loading || loadingLines);
  const settling = $derived(loading || searching);

  const lineMark = $derived(pattern(asked, lineHow));

  const marks = $derived(
    [lineMark, filtering ? pattern(query, how) : null].filter(
      (one): one is RegExp => one !== null,
    ),
  );

  const trail = $derived(
    opened ? { under: opened.under, leaf: opened.name } : null,
  );

  const SPAN = 600;
  const STEP = 200;
  const COALESCE = 200;

  let sentinel = $state<HTMLLIElement | null>(null);
  let ceiling = $state<HTMLLIElement | null>(null);
  let jumping = $state(false);
  let turn = 0;

  const above = $derived(loaded?.from ?? 0);
  const below = $derived(loaded ? loaded.kept - loaded.from - lines.length : 0);

  $effect(() => {
    void lines;
    void editing;
    void viewTall;
    void above;
    void below;
    const node = box;
    if (!node) return;

    const measure = requestAnimationFrame(() => (listTall = node.scrollHeight));
    return () => cancelAnimationFrame(measure);
  });

  const sifted = (): Sift => ({
    show,
    only: wantedKind(),
    needle: asked,
    how: lineHow,
  });

  let refused = $state('');

  async function take(asking: (scope: string) => Promise<Outcome<Loaded>>) {
    const scope = opened?.scope;
    if (!scope) return;

    const mine = ++turn;
    loadingLines = loaded === null;

    const result = await caught(() => asking(scope));
    if (mine !== turn) return;

    loadingLines = false;
    if (result.status === 'error') {
      if (asked && lineMark === null) refused = result.error;
      else alarm(`${scope}: ${result.error}`);

      return;
    }

    refused = '';
    loaded = result.data;
  }

  async function look(from: number) {
    if (!opened?.scope) {
      loaded = null;
      return;
    }

    await take((scope) =>
      commands.readLines(gameDir, scope, sifted(), from, SPAN),
    );
  }

  async function around(file: string, id: number) {
    await take((scope) =>
      commands.readLinesAround(gameDir, scope, sifted(), file, id, SPAN),
    );
  }

  const drawn = () => [
    ...(box?.querySelectorAll<HTMLElement>('li[data-line]') ?? []),
  ];

  async function slide(from: number) {
    if (!box || !loaded || from === loaded.from) return;

    const anchor = drawn().find(
      (one) => one.offsetTop + one.offsetHeight > box!.scrollTop,
    );
    const mark = anchor?.dataset.line;
    const gap = anchor ? anchor.offsetTop - box.scrollTop : 0;

    await look(from);
    await tick();

    const now = mark && drawn().find((one) => one.dataset.line === mark);
    if (now && box) box.scrollTop = now.offsetTop - gap;
  }

  function more() {
    if (below > 0) slide(above + STEP);
  }

  function earlier() {
    if (above > 0) slide(Math.max(0, above - STEP));
  }

  async function rewind() {
    await look(0);
    if (box) box.scrollTop = 0;
  }

  async function pick(wanted: Show) {
    if (show === wanted) {
      await again();
      return;
    }

    show = wanted;
    await rewind();
  }

  const openRow = $derived(opened ? rowOf(opened.scope) : undefined);
  const alive = $derived.by(() => {
    const scope = opened?.scope;

    return (
      scope !== undefined && [...app.busy].some((key) => within(scope, key))
    );
  });

  const piled = $derived(app.piles);
  const wantedKind = () => (piled ? only : 'yours');

  async function sieve(wanted: Only) {
    if (only === wanted) {
      await again();
      return;
    }

    only = wanted;
    await rewind();
  }

  export async function reload() {
    if (!gameDir) return;

    const order = await loadRows();
    if (!order) return;

    forgetCounts();
    onorder(order);

    if (opened && !rail.rows.some((one) => one.scope === opened?.scope))
      opened = null;
    else if (opened) await visit({ ...opened });
  }

  function enter(wanted: Viewing) {
    const reopening = opened?.scope === wanted.scope && loaded !== null;

    opened = wanted;
    if (!reopening) {
      editing = null;
      loaded = null;
      closeLineFilter();
    }

    return reopening;
  }

  async function visit(wanted: Viewing) {
    const from = enter(wanted) ? above : 0;

    await look(from);
  }

  function openedOf(row: Row): Viewing {
    const cut = row.label.lastIndexOf('/');

    return {
      scope: row.scope,
      name: cut < 0 ? row.label : row.label.slice(cut + 1),
      under: cut < 0 ? row.kind : row.label.slice(0, cut),
      files: row.tally.files,
    };
  }

  async function openRowAt(row: Row) {
    await visit(openedOf(row));
  }

  async function pickRow(row: Row) {
    const landing = onfile(row.scope);
    if (landing) {
      await reveal(landing.key, landing.id);
      return;
    }

    await openRowAt(row);
  }

  export async function reveal(key: string, id: number) {
    const wanted = rowOf(key);
    if (!wanted) return;

    if (opened?.scope !== wanted.scope) enter(openedOf(wanted));

    const there = () => lines.some((one) => one.file === key && one.id === id);

    if (!there()) {
      jumping = true;
      try {
        await around(key, id);
        if (!there() && (asked || show !== 'all' || wantedKind() !== 'yours')) {
          closeLineFilter();
          show = 'all';
          only = 'yours';
          await around(key, id);
        }
      } finally {
        jumping = false;
      }
    }

    if (!there()) return;

    await tick();
    document
      .getElementById(lineId({ file: key, id }))
      ?.scrollIntoView({ block: 'center' });
  }

  let pending: Record<string, { filled: number; added: number }> = {};
  let flushing: ReturnType<typeof setTimeout> | null = null;

  function forgetCounts() {
    pending = {};
    if (flushing) clearTimeout(flushing);
    flushing = null;
  }

  onDestroy(forgetCounts);

  export function recount(key: string, filled: number, added: number) {
    pending[key] = { filled, added: (pending[key]?.added ?? 0) + added };
    if (flushing) return;

    flushing = setTimeout(() => {
      flushing = null;
      const batch = pending;
      pending = {};
      for (const [file, count] of Object.entries(batch))
        counted(file, count.filled, count.added);
    }, COALESCE);
  }

  function counted(key: string, filled: number, added: number) {
    if (loaded && holding(key) && !asked && wantedKind() === 'yours') {
      const now =
        key === opened?.scope ? filled : loaded.counts.translated + added;
      loaded.counts.translated = now;
      loaded.counts.untranslated = loaded.counts.total - now;
    }

    credited(key, filled, added);
  }

  const holding = (key: string) => !!opened && within(opened.scope, key);

  async function again() {
    if (opened) await look(above);
  }

  export async function reread(key: string) {
    if (!holding(key)) return;
    await again();
  }

  const acts = scopeActions({
    reload,
    cleared: async (keys) => {
      if (keys.some(holding)) await again();
      await reload();
    },
  });

  function put(entry: Entry, value: string | null, path: string) {
    const was = isDone(entry);
    entry.translation = value;
    if (was === isDone(entry)) return;

    const by = isDone(entry) ? 1 : -1;
    if (loaded) {
      loaded.counts.translated += by;
      loaded.counts.untranslated -= by;
    }

    edited(path, by);
  }

  async function save(entry: Entry, value: string | null) {
    const wanted = value?.trim() ? value : null;
    if (wanted === entry.translation) return;

    const path = entry.file;
    const previous = entry.translation;

    put(entry, wanted, path);

    const result = await caught(() =>
      commands.saveEntry(gameDir, path, entry.id, wanted),
    );
    if (result.status === 'error') {
      put(entry, previous, path);
      alarm(`${path}: ${result.error}`);
    }
  }

  async function commit(entry: Entry, value: string, changed: boolean) {
    editing = null;
    if (changed) await save(entry, value);
  }

  async function beyond(at: number, by: 1 | -1) {
    const want = above + at + by;
    if (!loaded || want < 0 || want >= loaded.kept) return undefined;

    await look(by === 1 ? want : Math.max(0, want - SPAN + 1));

    return lines[want - above];
  }

  async function advance(
    entry: Entry,
    value: string,
    changed: boolean,
    by: 1 | -1,
  ) {
    const from = lineName(entry);
    const at = lines.findIndex((one) => lineName(one) === from);
    const near =
      by === 1
        ? (lines.slice(at + 1).find((one) => !isDone(one)) ?? lines[at + 1])
        : lines[at - 1];

    editing = near ? lineName(near) : null;
    if (changed) await save(entry, value);

    const next = near ?? (at < 0 ? undefined : await beyond(at, by));
    if (!next) {
      editing = null;
      return;
    }

    editing = lineName(next);

    await tick();
    document.getElementById(lineId(next))?.scrollIntoView({ block: 'nearest' });
  }

  async function suggest(entry: Entry): Promise<string | Error> {
    const result = await caught(() =>
      commands.translateLine(gameDir, entry.file, entry.id),
    );

    return result.status === 'ok' ? result.data : new Error(result.error);
  }

  async function clear(entry: Entry) {
    editing = null;
    await save(entry, null);
  }

  function closeLineFilter() {
    lineFilterOpen = false;
    lineFilter = '';
    asked = '';
    lineHow = { ...PLAIN };
  }

  async function stopLineFilter() {
    const was = asked;
    closeLineFilter();
    if (was) await rewind();
  }

  $effect(() => {
    const want = lineFilter;
    if (want === untrack(() => asked)) return;

    const timer = setTimeout(() => {
      asked = want;
      void rewind();
    }, 150);

    return () => clearTimeout(timer);
  });

  async function reseek() {
    asked = lineFilter;
    await rewind();
  }

  $effect(() => {
    const dir = gameDir;
    if (dir) untrack(() => reload());
  });

  $effect(() => {
    const mark = sentinel;
    const root = box;
    if (!mark || !root || jumping) return;

    return watch(root, [mark], more, '800px');
  });

  $effect(() => {
    const mark = ceiling;
    const root = box;
    if (!mark || !root || jumping) return;

    return watch(root, [mark], earlier, '800px');
  });

  $effect(() => {
    if (!filtering || !listed.length) return;
    if (opened && matches[opened.scope]) return;

    const first = listed[0];
    untrack(() => openRowAt(first));
  });
</script>

<div
  class="grid h-full min-h-0 grid-cols-[clamp(14rem,25%,27rem)_minmax(0,1fr)] gap-px bg-line"
>
  <FileRail
    {listed}
    {matches}
    open={opened?.scope ?? null}
    {searching}
    {filtering}
    narrowed={byName !== null}
    bind:nameFilter
    bind:nameHow
    {railBottom}
    onopen={pickRow}
    {acts}
  />

  <section class="flex min-h-0 flex-col overflow-hidden bg-surface">
    <LineBar
      {trail}
      {counts}
      {loading}
      {refused}
      {show}
      {only}
      {piled}
      bind:filterOpen={lineFilterOpen}
      bind:filter={lineFilter}
      bind:how={lineHow}
      onshow={pick}
      ononly={sieve}
      onclose={stopLineFilter}
      onreseek={reseek}
    />

    <div class="group/scroll relative min-h-0 flex-1">
      <ScrollBar
        of="editor-lines"
        tall={listTall}
        view={viewTall}
        at={scrolled}
        onmove={(top) => box?.scrollTo({ top })}
      />

      <ol
        bind:this={box}
        id="editor-lines"
        bind:clientHeight={viewTall}
        class="h-full overflow-auto bg-surface px-3 pb-6 {many ? '' : 'pt-2'}"
        onscroll={(event) => {
          scrolled = event.currentTarget.scrollTop;
          listTall = event.currentTarget.scrollHeight;
        }}
      >
        {#if above}
          <li
            bind:this={ceiling}
            class="pb-4 text-center text-xs text-ink-faint tabular-nums"
          >
            {above} earlier
          </li>
        {/if}

        {#each lines as entry, at (`${entry.file}/${entry.id}`)}
          {#if many && (at === 0 || lines[at - 1].file !== entry.file)}
            <li
              class="sticky top-0 z-10 -mx-3 bg-surface px-5 pt-3 pb-1 font-mono text-[11px] text-ink-faint"
            >
              {entry.name}
            </li>
          {/if}

          <EntryRow
            {entry}
            {marks}
            now={spot?.key === entry.file && spot?.id === entry.id}
            editing={editing === lineName(entry)}
            quiet={editing !== null}
            onbegin={() => (editing = lineName(entry))}
            onask={() => suggest(entry)}
            onhalt={() => void commands.stopLine()}
            oncommit={(value, changed) => commit(entry, value, changed)}
            onadvance={(value, changed, by) =>
              advance(entry, value, changed, by)}
            ondiscard={() => (editing = null)}
            onremove={() => clear(entry)}
          />
        {:else}
          {#if settling}
            {#each Array(7) as _unused, index (index)}
              <li
                class="grid animate-pulse grid-cols-2 gap-4 rounded-md px-2 py-1.5"
              >
                <span
                  class="flex min-h-[1.625em] items-center px-2 py-1 text-[13px]"
                >
                  <span
                    class="block h-2.5 rounded-full bg-sunken"
                    style="width: {60 + ((index * 17) % 35)}%"
                  ></span>
                </span>
                <span
                  class="flex min-h-[1.625em] items-center px-2 py-1 text-[13px]"
                >
                  <span
                    class="block h-2.5 rounded-full bg-sunken"
                    style="width: {45 + ((index * 11) % 40)}%"
                  ></span>
                </span>
              </li>
            {/each}
          {:else if !opened}
            <li class="flex h-full flex-col items-center justify-center gap-3">
              <BlankState
                Icon={MousePointerClick}
                said="Pick a file to see its lines"
              />
            </li>
          {:else}
            <li class="flex h-full flex-col items-center justify-center gap-3">
              {#if lineFilter}
                <BlankState Icon={SearchX} said="No matching line" />
              {:else if show === 'translated'}
                <BlankState Icon={Languages} said="No translated lines yet" />
              {:else if show === 'untranslated'}
                <BlankState Icon={Check} said="Everything is translated" />
              {:else if only !== 'yours'}
                <BlankState
                  Icon={ListFilter}
                  said={only === 'asked' ? NO_ASKED : NO_LISTED}
                />
              {:else}
                <BlankState
                  Icon={FileText}
                  said={many
                    ? 'These files have no text'
                    : 'This file has no text'}
                />
              {/if}
            </li>
          {/if}
        {/each}

        {#if below}
          <li
            bind:this={sentinel}
            class="pt-4 text-center text-xs text-ink-faint tabular-nums"
          >
            {below} more
          </li>
        {/if}
      </ol>
    </div>

    {#if opened}
      <ScopeBar scope={opened.scope} row={openRow} {alive} {acts} />
    {/if}
  </section>
</div>
