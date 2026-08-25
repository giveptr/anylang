<script lang="ts">
  import { Check, ChevronDown } from '@lucide/svelte';
  import Dropdown from '$lib/components/dropdown.svelte';
  import { byLabel } from '$lib/seek';

  type Option = { value: string; label: string; hint?: string };

  let {
    value = $bindable(),
    options,
    placeholder = '',
    searchable = true,
    clearable = false,
    onpick,
  }: {
    value: string;
    options: Option[];
    placeholder?: string;
    searchable?: boolean;
    clearable?: boolean;
    onpick?: (chosen: string) => void;
  } = $props();

  let open = $state(false);
  let needle = $state('');

  const shown = $derived(byLabel(options, needle));

  const chosen = $derived(
    options.find(
      (one) => one.value.toLowerCase() === (value ?? '').toLowerCase(),
    ),
  );

  function pick(next: string) {
    open = false;
    const settled = clearable && next === value ? '' : next;

    if (onpick) onpick(settled);
    else value = settled;
  }
</script>

<Dropdown
  bind:open
  bind:needle
  {searchable}
  onenter={() => shown[0] && pick(shown[0].value)}
>
  {#snippet trigger()}
    <span class="flex min-w-0 flex-1 items-baseline gap-2">
      <span class="shrink-0 {value ? '' : 'text-ink-faint'}">
        {chosen?.label ?? (value || placeholder)}
      </span>
      {#if chosen?.hint}
        <span class="min-w-0 flex-1 truncate text-ink-faint">{chosen.hint}</span
        >
      {/if}
    </span>
    <ChevronDown class="size-4 shrink-0 text-ink-faint" />
  {/snippet}

  {#each shown as one (one.value)}
    <li>
      <button type="button" class="option" onclick={() => pick(one.value)}>
        <span class="shrink-0">{one.label}</span>
        {#if one.hint}
          <span class="min-w-0 flex-1 truncate text-ink-faint">{one.hint}</span>
        {:else}
          <span class="flex-1"></span>
        {/if}
        {#if one === chosen}
          <Check class="size-3.5 shrink-0 text-accent" />
        {/if}
      </button>
    </li>
  {:else}
    <li class="px-3 py-4 text-center text-xs text-ink-faint">No match</li>
  {/each}
</Dropdown>
