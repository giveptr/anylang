<script lang="ts">
  import { ArrowRight } from '@lucide/svelte';
  import { open } from '@tauri-apps/plugin-dialog';
  import { app } from '$lib/app.svelte';
  import {
    drawn,
    sent,
    sendAllTo,
    sendTo,
    sentTo,
    show,
    styled,
  } from '$lib/fonts.svelte';
  import FileField from '$lib/components/file-field.svelte';
  import PathTail from '$lib/components/path-tail.svelte';

  const SAMPLE = 'The quick brown fox jumps over the lazy dog';
  const FONTS = [{ name: 'Font', extensions: ['ttf', 'otf'] }];

  let typed = $state('');
  let box = $state<HTMLInputElement | null>(null);
  const sample = $derived(typed.trim() || SAMPLE);

  $effect(() => box?.focus());

  const faces = $derived(app.faces);

  const seen = new WeakSet<Element>();
  const watching = new WeakMap<Element, string[]>();
  const watcher =
    typeof IntersectionObserver === 'undefined'
      ? null
      : new IntersectionObserver((hits) => {
          for (const hit of hits) {
            if (!hit.isIntersecting) continue;

            seen.add(hit.target);
            for (const at of watching.get(hit.target) ?? []) void show(at);
          }
        });

  function loads(node: HTMLElement, wanted: string[]) {
    const held = wanted.filter(Boolean);
    watching.set(node, held);

    if (watcher) watcher.observe(node);
    else for (const at of held) void show(at);

    return {
      update(next: string[]) {
        const held = next.filter(Boolean);
        watching.set(node, held);

        if (!watcher || seen.has(node)) for (const at of held) void show(at);
      },
      destroy() {
        watching.delete(node);
        watcher?.unobserve(node);
      },
    };
  }

  async function replaceAll() {
    const chosen = await open({ multiple: false, filters: FONTS });
    if (typeof chosen === 'string') sendAllTo(chosen);
  }
</script>

<div class="@container flex flex-col gap-4">
  <div class="flex items-center gap-2">
    <input
      class="bare-input"
      placeholder={SAMPLE}
      bind:this={box}
      bind:value={typed}
      aria-label="Sample text"
    />

    <div class="ml-auto flex shrink-0 items-center gap-2">
      {#if sent() > 0}
        <button
          class="rounded-lg px-3 py-2 text-sm text-ink-soft ring-1 ring-edge transition-colors hover:bg-sunken hover:text-ink"
          onclick={() => sendAllTo('')}
        >
          Clear every pick
        </button>
      {/if}
      <button
        class="rounded-lg px-3 py-2 text-sm ring-1 ring-edge transition-colors hover:bg-sunken"
        onclick={() => void replaceAll()}
      >
        Replace every font
      </button>
    </div>
  </div>

  <div class="flex flex-col">
    {#each faces as one (`${one.name}|${one.at}`)}
      {@const to = sentTo(one.name)}
      {@const face = one.shown}
      <div
        use:loads={[face, to]}
        class="grid gap-x-6 gap-y-3 border-t border-line py-4 @2xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]"
      >
        <div class="flex min-w-0 flex-col gap-2">
          <div class="flex min-h-9 items-baseline gap-2">
            <span class="min-w-0 truncate text-sm text-ink">
              {one.name}
            </span>
            {#if one.at}
              <span class="min-w-0 flex-1 text-xs text-ink-faint">
                <PathTail path={one.at} />
              </span>
            {/if}
            {#if one.builtin}
              <span class="ml-auto shrink-0 text-[11px] text-ink-faint">
                engine
              </span>
            {/if}
          </div>
          <p
            class="overflow-hidden text-xl leading-snug whitespace-nowrap text-ink"
            style={styled(face, one.name)}
          >
            {sample}
          </p>
        </div>

        <div class="flex min-w-0 flex-col gap-2">
          <div class="flex items-center gap-2">
            <ArrowRight class="size-3.5 shrink-0 text-ink-faint" />
            <FileField
              value={to}
              filters={FONTS}
              onpick={(picked) => sendTo(one.name, picked)}
            />
          </div>
          {#if to && drawn(to)}
            <p
              class="overflow-hidden text-xl leading-snug whitespace-nowrap text-ink"
              style={styled(to)}
            >
              {sample}
            </p>
          {/if}
        </div>
      </div>
    {/each}
  </div>
</div>
