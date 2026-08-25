<script lang="ts">
  import { ScrollText } from '@lucide/svelte';
  import { app } from '$lib/app.svelte';
  import { log, wipe } from '$lib/log.svelte';
  import { clockOf } from '$lib/format';
  import BlankState from '$lib/components/blank-state.svelte';

  let box = $state<HTMLDivElement | null>(null);
  let pinned = $state(true);

  const PAGE = 1000;

  let shown = $state(PAGE);

  const from = $derived(Math.max(0, log.lines.length - shown));
  const lines = $derived(log.lines.slice(from));

  function onscroll() {
    if (!box) return;
    pinned = box.scrollHeight - box.scrollTop - box.clientHeight < 24;
  }

  $effect(() => {
    const arrived = log.lines.length;
    if (arrived && pinned && box) box.scrollTop = box.scrollHeight;
  });
</script>

{#if log.lines.length}
  <div class="flex h-full flex-col gap-3">
    <div class="flex shrink-0 items-baseline gap-2 text-xs">
      <span class="text-ink-faint tabular-nums">
        {log.lines.length} line{log.lines.length === 1 ? '' : 's'}
      </span>

      {#if log.warnings}
        <span class="text-ink-faint">·</span>
        <span class="text-pending tabular-nums">
          {log.warnings} warning{log.warnings === 1 ? '' : 's'}
        </span>
      {/if}

      {#if log.errors}
        <span class="text-ink-faint">·</span>
        <span class="text-alarm tabular-nums">
          {log.errors} error{log.errors === 1 ? '' : 's'}
        </span>
      {/if}

      <button
        class="ml-auto text-ink-soft hover:text-ink"
        onclick={() => void wipe(app.gameDir)}
      >
        Clear
      </button>
    </div>

    <div bind:this={box} {onscroll} class="min-h-0 flex-1 overflow-auto">
      <ol class="flex flex-col gap-2">
        {#if from}
          <li class="flex justify-center pb-1">
            <button
              class="text-xs text-ink-soft hover:text-ink"
              onclick={() => (shown += PAGE)}
            >
              Show {from} earlier line{from === 1 ? '' : 's'}
            </button>
          </li>
        {/if}

        {#each lines as line (line)}
          {#if line.source === 'session' && lines[0] !== line}
            <li aria-hidden="true" class="my-1 h-px bg-line"></li>
          {/if}
          <li
            class="flex items-baseline gap-3 border-l-2 py-0.5 pl-3 font-mono text-xs {line.level ===
            'error'
              ? 'border-close'
              : line.level === 'warn'
                ? 'border-pending'
                : 'border-line'}"
          >
            <span class="shrink-0 text-[11px] text-ink-faint tabular-nums">
              {clockOf(line.at)}
            </span>
            <span class="min-w-40 shrink-0 text-[11px] text-ink-faint">
              {line.source}
            </span>
            <span
              class="min-w-0 flex-1 wrap-break-word whitespace-pre-wrap {line.level ===
              'error'
                ? 'text-alarm'
                : line.level === 'warn'
                  ? 'text-pending'
                  : 'text-ink-soft'}"
            >
              {line.message}
            </span>
          </li>
        {/each}
      </ol>
    </div>
  </div>
{:else}
  <div class="flex h-full flex-col items-center justify-center gap-3">
    <BlankState Icon={ScrollText} said="Nothing to report yet" />
  </div>
{/if}
