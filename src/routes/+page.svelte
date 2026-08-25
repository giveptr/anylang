<script lang="ts">
  import { Check, Download, Square } from '@lucide/svelte';
  import { onMount, untrack } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import {
    commands,
    events,
    type Found,
    type Opened,
    type Seeking,
  } from '$lib/bindings';
  import { caught, saveProject, tried } from '$lib/save';
  import { alarm, app } from '$lib/app.svelte';
  import {
    abandon,
    adopt,
    alone,
    halted,
    readGame,
    ready,
    resurvey,
    stopScope,
  } from '$lib/project';
  import { APPLY, NOTHING_TO_APPLY, TRANSLATE } from '$lib/wording';
  import { setShipped, shippedOf } from '$lib/sources';
  import { PLAIN, longEnough, pattern, same } from '$lib/seek';
  import { listen } from '$lib/log.svelte';
  import { finding, remember } from '$lib/finding.svelte';
  import { WHOLE, climb, within } from '$lib/scope';
  import { percent } from '$lib/format';
  import { swapped } from '$lib/pictures.svelte';
  import { sent } from '$lib/fonts.svelte';
  import { settle } from '$lib/rail.svelte';
  import DropZone from '$lib/components/drop-zone.svelte';
  import SettingsPage from '$lib/components/settings-page.svelte';
  import PicturesPage from '$lib/components/pictures-page.svelte';
  import Editor from '$lib/components/editor.svelte';
  import FindBar from '$lib/components/find-bar.svelte';
  import Notice from '$lib/components/notice.svelte';
  import SourceChoice from '$lib/components/source-choice.svelte';

  const SETTLE = 200;

  let viewer = $state<Editor | null>(null);
  let query = $state('');
  let asked = $state('');
  let how = $state<Seeking>({ ...PLAIN });
  let found = $state<Found[]>([]);
  let searching = $state(false);
  let at = $state(-1);
  let anchor = $state(0);
  let rowOrder = $state<string[]>([]);

  const filtering = $derived(
    finding.open && longEnough(asked) && pattern(asked, how) !== null,
  );

  const walk = $derived.by(() => {
    const place = new Map(rowOrder.map((scope, at) => [scope, at]));
    const rank = (key: string) =>
      climb(key, (at) => place.get(at)) ?? place.size;

    return [...found]
      .sort((a, b) => rank(a.key) - rank(b.key))
      .flatMap((one) => one.lines.map((id) => ({ key: one.key, id })));
  });
  const spot = $derived(at >= 0 ? (walk[at] ?? null) : null);

  const survey = $derived(app.survey);
  const left = $derived(survey ? survey.total - survey.translated : 0);
  const done = $derived(percent(survey?.translated ?? 0, survey?.total ?? 0));

  let picking = false;
  let choosing = $state<string[] | null>(null);

  async function begin(folder: string) {
    if (app.project) setShipped(app.project, folder);

    choosing = null;
    if (!(await readGame(true))) await abandon();
  }

  async function pick(dropped: string) {
    if (picking) return;
    picking = true;

    try {
      const opened = await tried(() => commands.pickGame(dropped));
      if (opened.status === 'error') return;

      adopt(opened.data);
      await resume(opened.data);
    } finally {
      picking = false;
    }
  }

  async function resume(opened: Opened) {
    if (opened.survey) return await refresh();

    if (opened.fresh && opened.sources.length) {
      choosing = opened.sources;
      return;
    }

    if (!(await readGame(false))) await abandon();
  }

  async function step(by: number) {
    if (!walk.length) return;

    const from = anchor % walk.length;
    const next =
      at < 0
        ? by > 0
          ? from
          : (from - 1 + walk.length) % walk.length
        : (at + by + walk.length) % walk.length;

    at = next;
    await viewer?.reveal(walk[next].key, walk[next].id);
  }

  function reorder(scopes: string[]) {
    const changed =
      scopes.length !== rowOrder.length ||
      scopes.some((one, where) => one !== rowOrder[where]);
    if (!changed) return;

    rowOrder = scopes;
    at = -1;
    anchor = 0;
  }

  function anchorOn(key: string) {
    const first = walk.findIndex((one) => within(key, one.key));
    if (first < 0) return null;

    const standing = at >= 0 && within(key, walk[at]?.key ?? '');

    anchor = first;
    if (!standing) at = first;

    return walk[at] ?? null;
  }

  function readFrom() {
    return app.project ? shippedOf(app.project) : '';
  }

  async function applyProject(was: string) {
    if (!app.project || !app.gameDir) return;

    if (!(await saveProject())) return;
    if (!app.project || !app.gameDir) return;

    if (readFrom() !== was) {
      query = '';
      resetMatches();
      await readGame(true);
      return;
    }

    await refresh();
  }

  async function refresh() {
    await resurvey();
    await viewer?.reload();
  }

  const worthApplying = $derived(
    (survey?.translated ?? 0) > 0 || swapped() > 0 || sent() > 0,
  );

  let exporting = $state(false);
  let justExported = $state(false);
  let exportedTimer: ReturnType<typeof setTimeout> | null = null;

  async function translate() {
    if (halted() || !ready()) return;

    await tried(() => commands.translateScope(app.gameDir, WHOLE));
    await refresh();
  }

  function stop() {
    stopScope(WHOLE);
  }

  async function exportGame() {
    if (!app.gameDir || exporting || halted() || !ready()) return;

    exporting = true;
    try {
      await alone(
        () => commands.exportScope(app.gameDir, WHOLE),
        (done) => {
          justExported = true;
          if (exportedTimer) clearTimeout(exportedTimer);
          exportedTimer = setTimeout(() => (justExported = false), 2000);
          settle(done);
        },
      );
    } finally {
      exporting = false;
    }
  }

  $effect(() => {
    remember(finding.open);
  });

  function resetMatches(to: Found[] = []) {
    found = to;
    at = -1;
    anchor = 0;
  }

  async function hunt(needle: string, asking: Seeking) {
    if (!app.gameDir) {
      searching = false;
      return;
    }

    const result = await caught(() =>
      commands.search(app.gameDir, needle, asking),
    );
    if (needle !== asked || !same(asking, how)) return;

    if (result.status === 'error') {
      alarm(result.error);
      searching = false;
      return;
    }

    if (result.data === null) {
      searching = false;
      return;
    }

    resetMatches(result.data);
    searching = false;

    if (walk.length) await step(1);
  }

  $effect(() => {
    if (app.gameDir) return;

    resetMatches();
    query = '';
    choosing = null;
  });

  $effect(() => {
    const want = finding.open ? query.trim() : '';
    if (want === untrack(() => asked)) return;

    const timer = setTimeout(() => (asked = want), SETTLE);
    return () => clearTimeout(timer);
  });

  $effect(() => {
    const needle = asked;
    const asking = { ...how };

    if (!longEnough(needle) || pattern(needle, asking) === null) {
      resetMatches();
      searching = false;
      return;
    }

    searching = true;
    void hunt(needle, asking);
  });

  let entered = '';
  let enteredRead = '';

  $effect(() => {
    const inSettings = app.view === 'settings';
    const now = untrack(() => JSON.stringify($state.snapshot(app.project)));

    if (inSettings) {
      entered = now;
      enteredRead = untrack(readFrom);
      return;
    }

    if (entered && now !== entered) applyProject(enteredRead);
    entered = '';
  });

  async function restore() {
    if (picking) return;
    picking = true;

    try {
      const now = await tried(() => commands.live());
      if (now.status === 'error') return;

      app.running = now.data.running;
      for (const file of now.data.files) app.busy.add(file);

      if (now.data.failed) {
        alarm(now.data.failed);
        return;
      }

      const opened = now.data.opened;
      if (!opened) return;

      adopt(opened);
      await resume(opened);
    } finally {
      picking = false;
    }
  }

  onMount(() => {
    restore().catch((error) => alarm(String(error)));

    const listeners = [
      events.preparing.listen(({ payload }) => {
        app.preparing = payload;
      }),
      events.batchDone.listen(({ payload }) =>
        viewer?.recount(payload.file, payload.filled, payload.added),
      ),
      events.runState.listen(({ payload }) => {
        app.running = payload.running;
        if (!payload.running) app.busy.clear();
      }),
      events.fileStarted.listen(({ payload }) => app.busy.add(payload.file)),
      events.fileDone.listen(({ payload }) => {
        app.busy.delete(payload.file);
        viewer?.reread(payload.file);
      }),
      listen(),
      getCurrentWindow().onCloseRequested(async () => {
        await saveProject();
      }),
    ];

    return () => {
      if (exportedTimer) clearTimeout(exportedTimer);
      for (const listener of listeners) listener.then((stop) => stop());
    };
  });
</script>

{#if app.preparing}
  {@const plan = app.preparing}
  <div class="grid h-full place-items-center px-6">
    {#if plan.steps.length === 0}
      <span
        class="size-6 animate-spin rounded-full border-2 border-accent-wash border-t-accent"
      ></span>
    {:else}
      <ol class="flex w-fit flex-col">
        {#each plan.steps as label, index (label)}
          <li class="relative flex items-center gap-3 pb-7 last:pb-0">
            {#if index < plan.steps.length - 1}
              <span
                class="absolute top-6 bottom-1 left-2.5 w-px -translate-x-1/2 {index <
                plan.at
                  ? 'bg-done'
                  : 'bg-line'}"
              ></span>
            {/if}

            {#if index < plan.at}
              <span
                class="grid size-5 shrink-0 place-items-center rounded-full bg-done text-on-accent"
              >
                <Check class="size-3" />
              </span>
            {:else if index === plan.at}
              <span
                class="size-5 shrink-0 animate-spin rounded-full border-2 border-accent-wash border-t-accent"
              ></span>
            {:else}
              <span class="size-5 shrink-0 rounded-full border-2 border-line"
              ></span>
            {/if}

            <span
              class="text-sm whitespace-nowrap {index === plan.at
                ? 'font-medium'
                : 'text-ink-faint'}"
            >
              {label}
            </span>
          </li>
        {/each}
      </ol>
    {/if}
  </div>
{:else if choosing}
  <SourceChoice sources={choosing} onread={begin} />
{:else if !survey}
  <DropZone onpick={pick} />
{:else}
  <div class="flex h-full min-h-0 flex-col">
    <Notice />

    <div
      class="flex min-h-0 flex-1 flex-col"
      class:hidden={app.view !== 'text'}
    >
      <FindBar
        bind:open={finding.open}
        bind:query
        bind:how
        total={walk.length}
        files={found.length}
        {at}
        onstep={step}
        {searching}
      />

      <div class="min-h-0 flex-1">
        <Editor
          bind:this={viewer}
          gameDir={app.gameDir}
          {found}
          query={asked}
          {how}
          {filtering}
          {searching}
          {spot}
          onorder={reorder}
          onfile={anchorOn}
        >
          {#snippet railBottom(loading: boolean)}
            <div class="flex flex-col gap-2.5">
              <div class="flex flex-col gap-1.5">
                <div class="flex items-baseline justify-between text-xs">
                  <span class="text-ink-soft tabular-nums">
                    {survey.translated} / {survey.total}
                  </span>

                  <span class="text-ink-faint tabular-nums">
                    {done}%
                  </span>
                </div>

                <div class="h-1 overflow-hidden rounded-full bg-sunken">
                  <div
                    class="h-full rounded-full transition-[width] {left === 0
                      ? 'bg-done'
                      : 'bg-accent'}"
                    style="width: {done}%"
                  ></div>
                </div>
              </div>

              <div class="flex gap-1.5">
                {#if app.running}
                  <button
                    class="flex flex-1 items-center justify-center gap-2 rounded-lg bg-sunken px-4 py-2 text-sm font-medium text-ink transition-colors hover:bg-alarm-wash hover:text-alarm"
                    onclick={stop}
                  >
                    <Square class="size-3.5" />
                    Stop all
                  </button>
                {:else}
                  <button
                    class="flex flex-1 items-center justify-center gap-2 rounded-lg bg-accent-strong px-4 py-2 text-sm font-semibold whitespace-nowrap text-on-accent transition-colors hover:bg-accent-deep disabled:bg-sunken disabled:font-medium disabled:text-ink-faint"
                    disabled={loading || app.working || left === 0}
                    onclick={translate}
                  >
                    {#if survey.files === 0}
                      Nothing chosen
                    {:else if survey.total === 0}
                      No text
                    {:else if left === 0}
                      Nothing left
                    {:else}
                      {TRANSLATE}
                    {/if}
                  </button>
                {/if}

                <button
                  class="flex shrink-0 items-center gap-1.5 rounded-lg px-3 py-2 text-sm font-medium whitespace-nowrap ring-1 ring-edge transition-colors hover:bg-sunken disabled:text-ink-faint disabled:opacity-60 disabled:hover:bg-transparent"
                  aria-label={worthApplying ? APPLY : NOTHING_TO_APPLY}
                  disabled={loading ||
                    app.working ||
                    app.running ||
                    exporting ||
                    !worthApplying}
                  onclick={exportGame}
                >
                  {#if exporting}
                    <span
                      class="size-3.5 shrink-0 animate-spin rounded-full border border-ink-faint border-t-transparent"
                    ></span>
                  {:else if justExported}
                    <Check class="size-3.5 shrink-0" />
                  {:else}
                    <Download class="size-3.5 shrink-0" />
                  {/if}
                  <span class="@max-[16rem]:hidden">
                    {worthApplying ? APPLY : NOTHING_TO_APPLY}
                  </span>
                </button>
              </div>
            </div>
          {/snippet}
        </Editor>
      </div>
    </div>

    {#if app.view === 'pictures'}
      <div class="min-h-0 flex-1"><PicturesPage /></div>
    {:else if app.view === 'settings'}
      <div class="min-h-0 flex-1"><SettingsPage /></div>
    {/if}
  </div>
{/if}
