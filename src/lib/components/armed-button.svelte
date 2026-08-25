<script lang="ts">
  import { getContext } from 'svelte';
  import type { Trash2 } from '@lucide/svelte';

  let {
    label,
    armedLabel,
    Icon,
    disabled = false,
    warning = '',
    compact = false,
    onfire,
  }: {
    label: string;
    armedLabel: string;
    Icon: typeof Trash2;
    disabled?: boolean;
    warning?: string;
    compact?: boolean;
    onfire: () => void;
  } = $props();

  const menu = getContext<{ close: () => void } | undefined>('menu');

  let arming = $state(false);

  export function disarm() {
    arming = false;
  }
</script>

<button
  type="button"
  {disabled}
  class="text-xs transition-colors disabled:text-ink-faint disabled:opacity-60 {compact
    ? 'flex shrink-0 items-center gap-1.5 rounded-md px-2 py-1 text-ink-soft'
    : 'flex w-full flex-col gap-1 px-3 py-1.5 text-left'} {arming
    ? 'bg-close font-medium text-on-accent'
    : 'enabled:hover:bg-sunken enabled:hover:text-alarm'}"
  onblur={() => (arming = false)}
  onclick={() => {
    if (!arming) {
      arming = true;
      return;
    }

    arming = false;
    onfire();
    menu?.close();
  }}
>
  {#if compact}
    <Icon class="size-3.5 shrink-0 opacity-70" />
    {arming ? armedLabel : label}
  {:else}
    <span class="flex w-full items-center justify-between gap-6">
      {arming ? armedLabel : label}
      <Icon class="size-3 shrink-0 opacity-70" />
    </span>
    {#if arming && warning}
      <span class="max-w-56 font-normal opacity-80">{warning}</span>
    {/if}
  {/if}
</button>
