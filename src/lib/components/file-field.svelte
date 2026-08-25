<script lang="ts">
  import { X } from '@lucide/svelte';
  import { open } from '@tauri-apps/plugin-dialog';
  import { fileName } from '$lib/format';

  type Props = {
    label?: string;
    value: string;
    filters?: { name: string; extensions: string[] }[];
    onpick: (chosen: string) => void;
  };

  let { label, value, filters, onpick }: Props = $props();

  async function browse() {
    const chosen = await open({ multiple: false, filters });
    if (typeof chosen === 'string') onpick(chosen);
  }
</script>

<div class="flex flex-col gap-1.5">
  {#if label}
    <span class="text-sm font-medium">{label}</span>
  {/if}
  <div class="flex min-w-0 items-center gap-2">
    <button
      type="button"
      onclick={browse}
      class="shrink-0 rounded-lg px-3 py-2 text-sm ring-1 ring-edge transition-colors hover:bg-sunken"
    >
      Choose
    </button>
    {#if value}
      <span class="min-w-0 truncate font-mono text-xs text-ink-faint">
        {fileName(value)}
      </span>
      <button
        type="button"
        class="grid size-6 shrink-0 place-items-center rounded-md text-ink-faint transition-colors hover:bg-sunken hover:text-ink"
        aria-label="Clear {label ?? 'the chosen file'}"
        onclick={() => onpick('')}
      >
        <X class="size-3.5" />
      </button>
    {/if}
  </div>
</div>
