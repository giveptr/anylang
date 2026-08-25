<script lang="ts">
  import { X } from '@lucide/svelte';
  import { fileName } from '$lib/format';
  import { replacementShown, showReplacement } from '$lib/pictures.svelte';
  import PictureFrame from '$lib/components/picture-frame.svelte';

  type Props = {
    at: string;
    wide: number;
    high: number;
    atlas: string;
    over: boolean;
    onclear: () => void;
    onzoom: () => void;
  };

  let { at, wide, high, atlas, over, onclear, onzoom }: Props = $props();

  $effect(() => {
    void showReplacement(at);
  });

  const held = $derived(replacementShown(at));
  const source = $derived(held?.ok ? held.source : '');
  const why = $derived(held && !held.ok ? held.error : '');

  const stretched = $derived(
    held !== null && held.ok && (held.wide !== wide || held.high !== high),
  );

  const resized = $derived.by(() => {
    if (!held?.ok) return '';

    const from = `${held.wide}×${held.high}`;
    const to = `${wide}×${high}`;

    if (held.wide < wide && held.high < high)
      return `Stretched from ${from} up to ${to}, so it will look soft.`;

    if (held.wide > wide && held.high > high)
      return `Shrunk from ${from} down to ${to}.`;

    return `Resized from ${from} to ${to}.`;
  });
</script>

<div class="flex flex-col gap-1.5">
  <div class="flex items-center gap-1.5">
    <span class="min-w-0 flex-1 truncate text-[11px] text-ink-faint">
      Replacement · {fileName(at)}
    </span>
    <button
      class="icon-button size-7 shrink-0"
      aria-label="Clear this pick"
      onclick={onclear}
    >
      <X class="size-3.5" />
    </button>
  </div>

  {#if why}
    <p class="rounded-md bg-alarm-wash px-3 py-2 text-xs text-alarm">{why}</p>
  {:else if source}
    <PictureFrame
      src={source}
      name={fileName(at)}
      ring="ring-2 transition-colors {over
        ? 'bg-accent-wash ring-accent'
        : 'ring-accent/40'}"
      {onzoom}
    />
  {:else}
    <div
      class="flex h-52 items-center justify-center overflow-hidden rounded-md checkers ring-2 transition-colors {over
        ? 'bg-accent-wash ring-accent'
        : 'ring-accent/40'}"
    ></div>
  {/if}

  {#if stretched}
    <p class="text-[11px] text-ink-faint">
      {resized}
      {#if atlas}
        Its spot in {atlas} cannot change size.
      {/if}
    </p>
  {/if}
</div>
