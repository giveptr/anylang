<script lang="ts">
  import { Sparkles, X } from '@lucide/svelte';
  import { untrack } from 'svelte';
  import type { Entry } from '$lib/bindings';
  import Highlight from '$lib/components/highlight.svelte';
  import Hint from '$lib/components/hint.svelte';
  import { LISTED_HINT } from '$lib/wording';
  import { lineId, lineName } from '$lib/components/types';

  let {
    entry,
    marks,
    now,
    editing,
    quiet,
    onbegin,
    onask,
    onhalt,
    oncommit,
    onadvance,
    ondiscard,
    onremove,
  }: {
    entry: Entry;
    marks: RegExp[];
    now: boolean;
    editing: boolean;
    quiet: boolean;
    onbegin: () => void;
    onask: () => Promise<string | Error>;
    onhalt: () => void;
    oncommit: (value: string, changed: boolean) => void;
    onadvance: (value: string, changed: boolean, by: 1 | -1) => void;
    ondiscard: () => void;
    onremove: () => void;
  } = $props();

  let draft = $state('');
  let changed = $state(false);
  let editor = $state<HTMLTextAreaElement | null>(null);
  let panel = $state<HTMLDivElement | null>(null);
  let above = $state(false);
  let asking = $state(false);
  let halted = false;
  let trouble = $state('');
  let session = 0;

  const guessed = $derived(entry.offer === 'listed');

  const hints = [
    ['Enter', 'save'],
    ['Shift+Enter', 'new line'],
    ['Esc', 'discard'],
  ] as const;

  function sized() {
    if (!editor) return;

    editor.style.height = 'auto';
    editor.style.height = `${editor.scrollHeight}px`;
  }

  function place() {
    const list = panel?.closest('ol');
    const row = panel?.parentElement;
    if (!panel || !list || !row) return;

    const room = list.getBoundingClientRect();
    const anchor = row.getBoundingClientRect();
    const height = panel.getBoundingClientRect().height;

    above =
      anchor.top + height > room.bottom && anchor.bottom - height >= room.top;
  }

  $effect(() => {
    if (!editing) return;

    session += 1;
    const start = untrack(() => entry.translation ?? entry.source);
    draft = start;
    changed = false;
    above = false;
    asking = false;
    trouble = '';

    queueMicrotask(() => {
      if (!editor) return;
      editor.focus();
      editor.setSelectionRange(start.length, start.length);
      sized();
      place();
    });
  });

  $effect(() => {
    if (!editing) return;

    const leave = (event: PointerEvent) => {
      if (!panel || panel.contains(event.target as Node)) return;
      oncommit(draft, changed);
    };

    window.addEventListener('pointerdown', leave);
    return () => window.removeEventListener('pointerdown', leave);
  });

  async function ask() {
    if (asking) return;

    trouble = '';
    halted = false;
    asking = true;
    const mine = session;
    const answer = await onask();
    if (mine !== session || !editing) return;

    asking = false;

    if (answer instanceof Error) {
      if (!halted) trouble = answer.message;
      return;
    }

    if (halted) return;

    draft = answer;
    changed = true;
    queueMicrotask(sized);
    editor?.focus();
  }

  function halt() {
    halted = true;
    onhalt();
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.isComposing) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      ondiscard();
    } else if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      ask();
    } else if (event.key === 'Tab') {
      event.preventDefault();
      onadvance(draft, changed, event.shiftKey ? -1 : 1);
    } else if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      onadvance(draft, changed, 1);
    }
  }
</script>

<li
  data-line={lineName(entry)}
  id={lineId(entry)}
  class="group relative grid scroll-mt-9 scroll-mb-2 grid-cols-2 gap-4 rounded-md px-2 py-1.5 {quiet
    ? ''
    : 'hover:bg-sunken'}"
>
  <p
    class="px-2 py-1 text-[13px] leading-relaxed whitespace-pre-wrap {guessed &&
    !entry.translation
      ? 'text-ink-faint'
      : 'text-ink-soft'}"
  >
    <span class="block min-h-[1.625em]"
      ><Highlight text={entry.source} {marks} {now} /></span
    >
  </p>

  <button
    type="button"
    onclick={onbegin}
    class="w-full cursor-text rounded-md px-2 py-1 text-left text-[13px] leading-relaxed whitespace-pre-wrap"
  >
    <span class="block min-h-[1.625em]">
      {#if entry.translation}
        <Highlight text={entry.translation} {marks} {now} />
      {:else}
        <span class="flex min-h-[1.625em] items-center">
          <span
            class="h-px w-24 rounded-full bg-line {quiet
              ? ''
              : 'group-hover:bg-ink-faint'}"
          ></span>
        </span>
      {/if}
    </span>
  </button>

  {#if entry.translation && !editing}
    <button
      type="button"
      aria-label="Remove this translation"
      class="pointer-events-none absolute top-1.5 right-1 z-20 rounded p-1 text-ink-faint opacity-0 transition-opacity hover:text-alarm focus-visible:opacity-100 {quiet
        ? ''
        : 'group-hover:pointer-events-auto group-hover:opacity-100'}"
      onclick={onremove}
    >
      <X class="size-3.5" />
    </button>
  {/if}

  {#if editing}
    <div
      bind:this={panel}
      class="absolute inset-x-0 z-30 flex flex-col gap-3 rounded-lg bg-surface p-4 shadow-xl ring-1 ring-accent {above
        ? 'bottom-0'
        : 'top-0'}"
    >
      {#if guessed}
        <p
          class="rounded-md bg-pending-wash px-2.5 py-1.5 text-[11px] leading-relaxed text-pending"
        >
          {LISTED_HINT}
        </p>
      {/if}

      <div class="flex items-start gap-3">
        <div class="flex min-w-0 flex-1 flex-col gap-1">
          <p
            class="max-h-[30vh] cursor-text overflow-y-auto text-[13px] leading-relaxed whitespace-pre-wrap text-ink-faint select-text"
          >
            <Highlight text={entry.source} {marks} />
          </p>
        </div>

        <Hint text="Ctrl+Enter">
          <button
            class="flex shrink-0 items-center gap-1.5 text-[11px] font-medium text-accent transition-colors hover:text-ink"
            onclick={asking ? halt : ask}
          >
            {#if asking}
              <span
                class="size-3 animate-spin rounded-full border border-accent-wash border-t-accent"
              ></span>
              Stop
            {:else}
              <Sparkles class="size-3" />
              Suggest
            {/if}
          </button>
        </Hint>
      </div>

      <textarea
        bind:this={editor}
        bind:value={draft}
        oninput={() => {
          changed = true;
          sized();
        }}
        {onkeydown}
        rows="4"
        class="max-h-[45vh] w-full resize-none overflow-y-auto bg-transparent text-[13px] leading-relaxed outline-none"
      ></textarea>

      {#if trouble}
        <p
          class="rounded-md bg-alarm-wash px-2.5 py-1.5 text-[11px] leading-relaxed text-alarm"
        >
          {trouble}
        </p>
      {/if}

      <div class="flex items-center gap-5 text-[11px] text-ink-soft">
        {#each hints as [key, what] (key)}
          <span class="flex items-center gap-1.5">
            <kbd
              class="rounded bg-sunken px-1.5 py-0.5 font-sans text-[10px] font-medium text-ink ring-1 ring-line"
            >
              {key}
            </kbd>
            {what}
          </span>
        {/each}

        {#if entry.translation}
          <button
            class="ml-auto rounded px-2 py-0.5 font-medium text-alarm hover:bg-alarm-wash"
            onclick={onremove}
          >
            Remove
          </button>
        {/if}
      </div>
    </div>
  {/if}
</li>
