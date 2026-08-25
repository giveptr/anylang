<script lang="ts">
  import { getContext } from 'svelte';
  import type { Undo2 } from '@lucide/svelte';

  let {
    label,
    Icon,
    act,
    off = false,
  }: {
    label: string;
    Icon: typeof Undo2;
    act: () => void;
    off?: boolean;
  } = $props();

  const menu = getContext<{ close: () => void } | undefined>('menu');
</script>

<button
  type="button"
  class="flex w-full items-center justify-between gap-6 px-3 py-1.5 text-left text-xs transition-colors hover:bg-sunken disabled:text-ink-faint disabled:opacity-60 disabled:hover:bg-transparent"
  disabled={off}
  onclick={() => {
    act();
    menu?.close();
  }}
>
  {label}
  <Icon class="size-3 shrink-0 opacity-70" />
</button>
