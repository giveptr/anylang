<script lang="ts">
  import { onMount } from 'svelte';
  import {
    Check,
    ChevronRight,
    Copy,
    Download,
    ImageOff,
    ImageUp,
    Search,
    SearchX,
    Star,
    X,
  } from '@lucide/svelte';
  import { getCurrentWebview } from '@tauri-apps/api/webview';
  import { open, save } from '@tauri-apps/plugin-dialog';
  import { about } from '$lib/about.svelte';
  import { alarm, app } from '$lib/app.svelte';
  import { commands, type Shot } from '$lib/bindings';
  import type { Wanted } from '$lib/drawing';
  import { fileName } from '$lib/format';
  import { NO_PIXELS } from '$lib/wording';
  import { tried } from '$lib/save';
  import {
    FULL,
    TILE,
    gallery,
    load,
    marked,
    replacementShown,
    shown,
    whyBlank,
    swapTo,
    swappedTo,
    swapped,
    toggleMark,
    want,
  } from '$lib/pictures.svelte';
  import PictureLightbox, {
    veiled,
  } from '$lib/components/picture-lightbox.svelte';
  import type { Narrow, Zoomed } from '$lib/components/types';
  import PictureDetail from '$lib/components/picture-detail.svelte';
  import ShelfRail from '$lib/components/shelf-rail.svelte';
  import PictureTile from '$lib/components/picture-tile.svelte';
  import PictureMenu from '$lib/components/picture-menu.svelte';
  import TileGrid from '$lib/components/tile-grid.svelte';
  import BlankState from '$lib/components/blank-state.svelte';
  import { type Branch, branches } from '$lib/branches';
  import { spotOf } from '$lib/shots';
  import { within } from '$lib/scope';

  const NARROWEST = 152;
  const LABELS = 58;

  const CHIP =
    'grid size-7 place-items-center rounded-full transition-colors hover:bg-sunken disabled:pointer-events-none disabled:opacity-40';
  const CHIP_PLAIN = 'text-ink-soft hover:text-ink';

  const EVERYTHING: Narrow = {
    query: '',
    spot: '',
    atlas: '',
    kept: 'every',
  };

  let sift = $state<Narrow>({ ...EVERYTHING });
  let opened = $state<Shot | null>(null);
  let zoomed = $state<Zoomed | null>(null);

  $effect(() => {
    if (!opened) zoomed = null;
  });
  let onscreen = $state<Shot[]>([]);
  let over = $state('');
  let copied = $state('');
  let finding = $state(false);
  let hunt = $state<HTMLInputElement | null>(null);
  let grid = $state<ReturnType<typeof TileGrid> | null>(null);
  let menu = $state<ReturnType<typeof PictureMenu> | null>(null);

  void load();

  const cutFrom = $derived(
    new Set(gallery.shots.filter((one) => one.atlas).map((one) => one.atlas)),
  );

  const holds = (one: Shot) => cutFrom.has(one.name);

  const pictures = $derived(gallery.shots.filter((one) => !holds(one)));

  const biggestFirst = (one: Branch<Shot>, other: Branch<Shot>) =>
    other.many - one.many || one.label.localeCompare(other.label);

  const shelves = $derived(branches(pictures, spotOf, biggestFirst));

  const atlasIn = (spot: string, atlas: string) =>
    gallery.shots.find(
      (one) => spotOf(one) === spot && one.name === atlas && holds(one),
    ) ?? null;

  const cutsIn = (one: Shot) => {
    if (!holds(one)) return 0;

    const spot = spotOf(one);

    return gallery.shots.filter(
      (held) => held.atlas === one.name && spotOf(held) === spot,
    ).length;
  };

  const needle = $derived(sift.query.trim().toLowerCase());
  const searching = $derived(needle.length > 0);

  const showing = $derived(
    (sift.kept === 'every' ? pictures : gallery.shots).filter((one) => {
      if (sift.kept === 'replaced' && !swappedTo(one.key)) return false;
      if (sift.kept === 'marked' && !marked(one.key)) return false;
      if (sift.spot && !within(sift.spot, spotOf(one))) return false;
      if (sift.atlas && one.atlas !== sift.atlas) return false;

      if (!needle) return true;

      return (
        one.name.toLowerCase().includes(needle) ||
        one.atlas.toLowerCase().includes(needle) ||
        (one.at ?? '').toLowerCase().includes(needle)
      );
    }),
  );

  const replacement = $derived(opened ? swappedTo(opened.key) : '');
  const swapHeld = $derived(replacement ? replacementShown(replacement) : null);
  const swapSource = $derived(swapHeld?.ok ? swapHeld.source : '');
  const swapWhy = $derived(swapHeld && !swapHeld.ok ? swapHeld.error : '');

  const stars = $derived(gallery.shots.filter((one) => marked(one.key)).length);

  const zooming = $derived.by(() => {
    const held = opened;
    if (!held || !zoomed) return null;

    if (zoomed === 'replacement')
      return { src: swapSource, name: fileName(replacement), why: swapWhy };

    return {
      src: held.drawable ? shown(held.key, FULL) : '',
      name: held.name,
      why: held.drawable ? whyBlank(held.key, FULL) : NO_PIXELS,
    };
  });

  const openedIn = $derived(opened ? spotOf(opened) : '');

  type Step = { label: string; at: string };

  const plain = (label: string): Step => ({ label, at: '' });

  function stepsTo(path: string): Step[] {
    if (!path) return [];

    const parts = path.split('/');

    return parts.map((one, at) => ({
      label: one,
      at: parts.slice(0, at + 1).join('/'),
    }));
  }

  const trail = $derived.by((): Step[] => {
    const held = opened;

    if (held) {
      const at = held.at ?? '';
      const tail = at.startsWith(`${openedIn}/`)
        ? at.slice(openedIn.length + 1)
        : held.name;

      return [...stepsTo(openedIn), ...tail.split('/').map(plain)];
    }

    if (sift.kept === 'replaced') return [plain('Replaced')];
    if (sift.kept === 'marked') return [plain('Marked')];
    if (!sift.spot) return [plain('Every picture')];

    return [...stepsTo(sift.spot), ...(sift.atlas ? [plain(sift.atlas)] : [])];
  });

  const wanted = $derived<Wanted[]>([
    ...(opened?.drawable ? [{ key: opened.key, most: FULL }] : []),
    ...onscreen
      .filter((one) => one.drawable)
      .map((one) => ({ key: one.key, most: TILE })),
  ]);

  $effect(() => want(wanted));

  $effect(() => {
    const held = opened;

    if (held && !gallery.shots.some((one) => one.key === held.key))
      opened = null;
  });

  function narrow(onto: Partial<Narrow>) {
    finding = false;
    sift = { ...EVERYTHING, ...onto };
    opened = null;
    grid?.reset();
  }

  function seek(query: string) {
    sift = { ...sift, query };
    grid?.reset();
  }

  function look(spot: string, atlas: string) {
    narrow({ spot, atlas });
    opened = atlas ? atlasIn(spot, atlas) : null;
  }

  function find() {
    finding = true;
    queueMicrotask(() => hunt?.select());
  }

  function shut() {
    finding = false;
    seek('');
  }

  const typingIn = (target: EventTarget | null) =>
    !!(target as HTMLElement | null)?.closest('input, textarea');

  const pickingWords = () => {
    const held = window.getSelection();

    return !!held && !held.isCollapsed;
  };

  function pressed(event: KeyboardEvent) {
    if (event.isComposing) return;

    if (event.key === 'Escape') {
      if (veiled() || menu?.shown()) return;

      if (finding) shut();
      else opened = null;

      return;
    }

    if (!(event.ctrlKey || event.metaKey) || typingIn(event.target)) return;

    const held = opened;

    switch (event.key) {
      case 's':
        event.preventDefault();
        if (held?.drawable) void takeCopy(held);
        return;
      case 'c':
        if (pickingWords()) return;

        event.preventDefault();
        if (held?.drawable) void copyOut(held);
        return;
      case 'm':
        event.preventDefault();
        if (held) toggleMark(held.key);
        return;
      case 'r':
        event.preventDefault();
        if (held) void choose(held);
        return;
      case 'v':
        event.preventDefault();
        if (held) void pasteIn(held);
        else alarm('Pick a picture first, then paste');
        return;
      case 'f':
        if (veiled()) return;

        event.preventDefault();
        find();
    }
  }

  function keyAt(spot: { x: number; y: number }) {
    const ratio = window.devicePixelRatio || 1;
    const held = document.elementFromPoint(spot.x / ratio, spot.y / ratio);

    return held?.closest('[data-key]')?.getAttribute('data-key') ?? '';
  }

  const aPicture = (at: string) =>
    about.kinds.some((kind) => at.toLowerCase().endsWith(`.${kind}`));

  function dropped(key: string, paths: string[]) {
    const [first] = paths.filter(aPicture);
    const shot = gallery.shots.find((one) => one.key === key);

    if (!first || !shot || shot.locked) return;

    opened = shot;
    void bring(shot, first);
  }

  onMount(() => {
    const stop = getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === 'leave') {
        over = '';
        return;
      }

      const key = keyAt(event.payload.position);

      if (event.payload.type === 'drop') {
        over = '';
        dropped(key, event.payload.paths);
        return;
      }

      over = key;
    });

    return () => void stop.then((remove) => remove());
  });

  async function bring(one: Shot, at: string) {
    const done = await tried(() => commands.keepPicture(app.gameDir, at));
    if (done.status !== 'ok') return;

    swapTo(one.key, done.data);
  }

  async function choose(one: Shot) {
    if (one.locked) {
      alarm(one.locked);
      return;
    }

    const at = await open({
      multiple: false,
      filters: [{ name: 'Image', extensions: about.kinds }],
    });
    if (typeof at !== 'string') return;

    await bring(one, at);
  }

  async function pasteIn(one: Shot) {
    if (one.locked) {
      alarm(one.locked);
      return;
    }

    const done = await tried(() => commands.pastePicture(app.gameDir));
    if (done.status !== 'ok') return;

    if (done.data) swapTo(one.key, done.data);
  }

  async function copyOut(one: Shot) {
    const done = await tried(() => commands.copyPicture(app.gameDir, one.key));
    if (done.status !== 'ok') return;

    copied = one.key;
    setTimeout(() => (copied = ''), 1200);
  }

  async function takeCopy(one: Shot) {
    const kind = one.savedAs || 'png';
    const named = one.name
      .replace(/[\\/]/g, '-')
      .replace(new RegExp(`\\.${kind}$`, 'i'), '');
    const at = await save({
      defaultPath: `${named}.${kind}`,
      filters: [{ name: kind.toUpperCase(), extensions: [kind] }],
    });
    if (typeof at !== 'string') return;

    await tried(() => commands.savePicture(app.gameDir, one.key, at));
  }
</script>

<svelte:window onkeydown={pressed} />

<div class="grid h-full min-h-0 grid-cols-[15rem_minmax(0,1fr)] gap-px bg-line">
  <ShelfRail
    {shelves}
    count={pictures.length}
    replaced={swapped()}
    marked={stars}
    loading={gallery.loading}
    {openedIn}
    {sift}
    onnarrow={narrow}
    onlook={look}
  />

  <div
    class="grid min-h-0 {showing.length === 0 && !gallery.loading
      ? 'grid-cols-1'
      : 'grid-cols-[minmax(0,1fr)_21rem]'} gap-px bg-line"
  >
    <div class="flex min-h-0 flex-col bg-surface">
      {#if finding}
        <div
          class="flex h-11 shrink-0 items-center gap-2.5 border-b border-line pr-3.5 pl-4.5"
        >
          <Search class="size-4 shrink-0 text-ink-faint" />

          <input
            bind:this={hunt}
            class="bare-input"
            placeholder="Search these pictures (Ctrl F)"
            value={sift.query}
            oninput={(event) => seek(event.currentTarget.value)}
            onkeydown={(event) => {
              if (event.isComposing || event.key !== 'Escape') return;

              event.stopPropagation();
              shut();
            }}
          />

          {#if searching}
            <span class="shrink-0 text-xs text-ink-soft tabular-nums">
              {#if showing.length === 0}
                no matches
              {:else}
                {showing.length} match{showing.length === 1 ? '' : 'es'}
              {/if}
            </span>
          {/if}

          <button
            class="icon-button size-7 shrink-0"
            aria-label="Close search"
            onclick={shut}
          >
            <X class="size-4" />
          </button>
        </div>
      {:else}
        <div
          class="flex h-11 shrink-0 items-center gap-1 border-b border-line pr-3 pl-4.5 text-xs text-ink-soft"
        >
          {#if gallery.loading}
            <span class="block h-2.5 w-32 animate-pulse rounded-full bg-sunken"
            ></span>
          {:else}
            {#each trail as step, index (index)}
              {#if index > 0}
                <ChevronRight class="size-3 shrink-0" />
              {/if}
              {#if step.at && index < trail.length - 1}
                <button
                  class="min-w-0 truncate rounded px-1 hover:bg-sunken hover:text-ink"
                  onclick={() => narrow({ spot: step.at })}
                >
                  {step.label}
                </button>
              {:else}
                <span
                  class={index === trail.length - 1
                    ? 'shrink-0 text-ink'
                    : 'min-w-0 truncate'}>{step.label}</span
                >
              {/if}
            {/each}

            <span class="min-w-0 flex-1"></span>
            <button
              class="icon-button size-7 shrink-0"
              aria-label="Find a picture by name or path"
              onclick={find}
            >
              <Search class="size-4" />
            </button>
          {/if}
        </div>
      {/if}

      {#if gallery.loading}
        <div
          class="grid animate-pulse grid-cols-[repeat(auto-fill,minmax(9.5rem,1fr))] content-start gap-1 p-3"
        >
          {#each Array(24) as _unused, index (index)}
            <div class="flex flex-col gap-1.5 p-2">
              <span class="block aspect-square w-full rounded-md bg-sunken"
              ></span>
              <span
                class="block h-2.5 rounded-full bg-sunken"
                style="width: {52 + ((index * 19) % 44)}%"
              ></span>
              <span class="block h-2 w-10 rounded-full bg-sunken"></span>
            </div>
          {/each}
        </div>
      {:else if showing.length === 0}
        <div
          class="flex min-h-0 flex-1 flex-col items-center justify-center gap-3"
        >
          {#if searching}
            <BlankState Icon={SearchX} said="No matching picture" />
          {:else if sift.kept === 'replaced'}
            <BlankState Icon={ImageUp} said="No picture replaced yet" />
          {:else if sift.kept === 'marked'}
            <BlankState Icon={Star} said="No picture marked yet" />
          {:else if sift.spot}
            <BlankState Icon={ImageOff} said="No picture here" />
          {:else}
            <BlankState Icon={ImageOff} said="No picture in this game" />
          {/if}
        </div>
      {:else}
        <TileGrid
          bind:this={grid}
          items={showing}
          narrowest={NARROWEST}
          labels={LABELS}
          named={(one) => one.key}
          onscreen={(cut) => (onscreen = cut)}
          chosen={opened}
          onchoose={(one) => (opened = one)}
        >
          {#snippet tile(shot)}
            <PictureTile
              {shot}
              over={over === shot.key}
              chosen={opened?.key === shot.key}
              onopen={(one) => {
                opened = one;
                grid?.take();
              }}
              onmenu={(event, one) => {
                opened = one;
                menu?.open(event, one);
              }}
            />
          {/snippet}
        </TileGrid>
      {/if}
    </div>

    {#if showing.length > 0 || gallery.loading}
      <PictureDetail
        shot={opened}
        loading={gallery.loading}
        {replacement}
        cuts={opened ? cutsIn(opened) : 0}
        over={!!over && over === opened?.key}
        copied={copied === opened?.key}
        onexport={(one) => void takeCopy(one)}
        oncopy={(one) => void copyOut(one)}
        onreplace={(one) => void choose(one)}
        onclear={(one) => swapTo(one.key, '')}
        onzoom={(which) => (zoomed = which)}
        onmark={(one) => toggleMark(one.key)}
      />
    {/if}
  </div>
</div>

<PictureMenu
  bind:this={menu}
  onexport={(one) => void takeCopy(one)}
  oncopy={(one) => void copyOut(one)}
  onreplace={(one) => void choose(one)}
  onpaste={(one) => void pasteIn(one)}
  onclear={(one) => swapTo(one.key, '')}
  onmark={(one) => toggleMark(one.key)}
/>

{#if zooming}
  {@const original = zoomed === 'original'}
  <PictureLightbox
    src={zooming.src}
    name={zooming.name}
    why={zooming.why}
    actions={original ? chips : undefined}
    onclose={() => (zoomed = null)}
    onkey={original ? (event) => grid?.step(event) : undefined}
  />
{/if}

{#snippet chips()}
  {#if opened}
    {@const held = opened}
    <button
      class="{CHIP} {CHIP_PLAIN}"
      aria-label="Export this picture to a file"
      disabled={!held.drawable}
      onclick={() => void takeCopy(held)}
    >
      <Download class="size-4" />
    </button>
    <button
      class="{CHIP} {CHIP_PLAIN}"
      aria-label="Copy this picture to the clipboard"
      disabled={!held.drawable}
      onclick={() => void copyOut(held)}
    >
      {#if copied === held.key}
        <Check class="size-4" />
      {:else}
        <Copy class="size-4" />
      {/if}
    </button>
    <button
      class="{CHIP} {marked(held.key) ? 'text-pending' : CHIP_PLAIN}"
      aria-label={marked(held.key)
        ? 'Unmark this picture'
        : 'Mark this picture'}
      onclick={() => toggleMark(held.key)}
    >
      <Star class="size-4 {marked(held.key) ? 'fill-current' : ''}" />
    </button>
  {/if}
{/snippet}
