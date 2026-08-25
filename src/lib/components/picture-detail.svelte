<script lang="ts">
  import {
    Check,
    Copy,
    Download,
    ImageUp,
    MousePointerClick,
    Star,
  } from '@lucide/svelte';
  import type { Shot } from '$lib/bindings';
  import { gauged } from '$lib/gauge';
  import { NO_PIXELS } from '$lib/wording';
  import type { Zoomed } from '$lib/components/types';
  import { FULL, marked, shown, whyBlank } from '$lib/pictures.svelte';
  import ReplacementPicture from '$lib/components/replacement-picture.svelte';
  import PictureFrame from '$lib/components/picture-frame.svelte';
  import BlankState from '$lib/components/blank-state.svelte';
  import ScrollBar from '$lib/components/scroll-bar.svelte';

  type Props = {
    shot: Shot | null;
    loading: boolean;
    replacement: string;
    cuts: number;
    over: boolean;
    copied: boolean;
    onexport: (shot: Shot) => void;
    oncopy: (shot: Shot) => void;
    onreplace: (shot: Shot) => void;
    onclear: (shot: Shot) => void;
    onzoom: (which: Zoomed) => void;
    onmark: (shot: Shot) => void;
  };

  let {
    shot,
    loading,
    replacement,
    cuts,
    over,
    copied,
    onexport,
    oncopy,
    onreplace,
    onclear,
    onzoom,
    onmark,
  }: Props = $props();

  let box = $state<HTMLDivElement | null>(null);
  let deep = $state<HTMLDivElement | null>(null);
  let view = $state(0);
  let at = $state(0);
  let long = $state(0);

  $effect(() =>
    gauged(box, deep, (tall, seen) => {
      long = tall;
      view = seen;
    }),
  );

  const told = $derived.by(() => {
    if (!shot) return '';

    const said = [`${shot.wide}×${shot.high}`];
    if (shot.format) said.push(shot.format);
    if (shot.atlas) said.push(`cut from ${shot.atlas}`);
    if (cuts > 0) said.push(`holds ${cuts} picture${cuts === 1 ? '' : 's'}`);

    return said.join(' · ');
  });
</script>

<aside
  data-key={shot?.key}
  class="flex min-h-0 flex-col overflow-hidden {over
    ? 'bg-accent-wash'
    : 'bg-surface'}"
>
  {#if shot}
    {@const held = shot}
    {@const whole = shown(held.key, FULL)}
    {@const starred = marked(held.key)}
    <div
      class="flex h-11 shrink-0 items-center gap-1 border-b border-line pr-3 pl-4"
    >
      <h2 class="min-w-0 flex-1 truncate text-sm font-medium">{held.name}</h2>
      {#if held.drawable}
        <button
          class="icon-button size-7 shrink-0"
          aria-label="Export this picture to a file"
          onclick={() => onexport(held)}
        >
          <Download class="size-4" />
        </button>
        <button
          class="icon-button size-7 shrink-0"
          aria-label="Copy this picture to the clipboard"
          onclick={() => oncopy(held)}
        >
          {#if copied}
            <Check class="size-4" />
          {:else}
            <Copy class="size-4" />
          {/if}
        </button>
      {/if}
      <button
        class="icon-button size-7 shrink-0 {starred ? 'text-pending' : ''}"
        aria-label={starred ? 'Unmark this picture' : 'Mark this picture'}
        onclick={() => onmark(held)}
      >
        <Star class="size-4 {starred ? 'fill-current' : ''}" />
      </button>
    </div>

    <div class="group/scroll relative min-h-0 flex-1">
      <ScrollBar
        of="picture-detail"
        tall={long}
        {view}
        {at}
        onmove={(top) => box?.scrollTo({ top })}
      />

      <div
        bind:this={box}
        id="picture-detail"
        onscroll={(event) => (at = event.currentTarget.scrollTop)}
        class="h-full overflow-auto p-4"
      >
        <div bind:this={deep} class="flex flex-col gap-4">
          <div class="flex flex-col gap-1.5">
            <span class="truncate text-[11px] text-ink-faint">
              {replacement ? `Original · ${told}` : told}
            </span>
            {#if held.drawable && whole}
              <PictureFrame
                src={whole}
                name={held.name}
                onzoom={() => onzoom('original')}
              />
            {:else}
              {@const why = held.drawable
                ? whyBlank(held.key, FULL)
                : NO_PIXELS}
              <div
                class="flex h-52 items-center justify-center overflow-hidden rounded-md checkers ring-1 ring-edge"
              >
                {#if why}
                  <span class="px-3 py-6 text-center text-xs text-ink-faint">
                    {why}
                  </span>
                {/if}
              </div>
            {/if}
          </div>

          {#if held.locked}
            <p class="rounded-md bg-sunken px-3 py-2 text-xs text-ink-soft">
              {held.locked}
            </p>
          {:else if replacement}
            <ReplacementPicture
              at={replacement}
              wide={held.wide}
              high={held.high}
              atlas={held.atlas}
              {over}
              onclear={() => onclear(held)}
              onzoom={() => onzoom('replacement')}
            />
          {/if}
        </div>
      </div>
    </div>

    {#if !held.locked && !replacement}
      <div class="shrink-0 border-t border-line p-4">
        <button
          class="flex w-full flex-col items-center gap-1.5 rounded-xl border-2 border-dashed px-4 py-4 text-center transition-colors {over
            ? 'border-accent bg-accent-wash'
            : 'border-line hover:bg-sunken'}"
          onclick={() => onreplace(held)}
        >
          <ImageUp
            class="size-5 transition-colors {over
              ? 'text-accent'
              : 'text-ink-faint'}"
            strokeWidth={1.5}
          />
          <span class="text-xs text-ink-faint">
            {over
              ? 'Let go to put it in'
              : 'Choose a picture, drop or paste one here'}
          </span>
        </button>
      </div>
    {/if}
  {:else if loading}
    <div class="flex animate-pulse flex-col gap-4">
      <div class="flex h-11 items-center border-b border-line pr-3 pl-4">
        <span class="block h-2.5 w-28 rounded-full bg-sunken"></span>
      </div>
      <div class="flex flex-col gap-1.5 px-4">
        <span class="block h-2 w-20 rounded-full bg-sunken"></span>
        <span class="block h-52 w-full rounded-md bg-sunken"></span>
      </div>
    </div>
  {:else}
    <div
      class="flex h-full flex-col items-center justify-center gap-3 p-6 text-center"
    >
      <BlankState
        Icon={MousePointerClick}
        said="Pick a picture to see it up close"
      />
    </div>
  {/if}
</aside>
