<script lang="ts">
  import { Check, ChevronDown, Plus, X } from '@lucide/svelte';
  import Dropdown from '$lib/components/dropdown.svelte';
  import { byLabel } from '$lib/seek';

  type Option = { value: string; label: string };

  let {
    values = $bindable(),
    options,
    placeholder,
  }: {
    values: string[];
    options: Option[];
    placeholder: string;
  } = $props();

  let needle = $state('');
  let shell = $state<ReturnType<typeof Dropdown> | null>(null);

  const labelOf = (value: string) =>
    options.find((one) => one.value === value)?.label ?? value;

  const shown = $derived(byLabel(options, needle));

  const custom = $derived.by(() => {
    const typed = needle.trim();
    if (!typed) return '';

    const known = options.some(
      (one) => one.label.toLowerCase() === typed.toLowerCase(),
    );
    const already = values.some(
      (value) => labelOf(value).toLowerCase() === typed.toLowerCase(),
    );
    return known || already ? '' : typed;
  });

  function settle() {
    needle = '';
    shell?.focus();
  }

  function toggle(value: string) {
    const list = [...values];
    const at = list.indexOf(value);
    if (at >= 0) list.splice(at, 1);
    else list.push(value);
    values = list;
    settle();
  }

  function add(value: string) {
    if (!values.includes(value)) values = [...values, value];
    settle();
  }
</script>

<Dropdown
  bind:this={shell}
  bind:needle
  placeholder="Search or type your own"
  layout="flex-wrap items-center gap-1.5 px-3 py-1.5"
  onenter={() => {
    if (custom) add(custom);
    else if (shown[0]) toggle(shown[0].value);
  }}
>
  {#snippet trigger()}
    {#each values as value (value)}
      <span
        class="flex items-center gap-1 rounded-full bg-accent-wash py-0.5 pr-1 pl-2.5 text-xs text-on-wash"
      >
        {labelOf(value)}
        <span
          role="button"
          tabindex="0"
          aria-label="Remove {labelOf(value)}"
          class="grid size-4 place-items-center rounded-full hover:bg-accent/15"
          onclick={(event) => {
            event.stopPropagation();
            toggle(value);
          }}
          onkeydown={(event) => event.key === 'Enter' && toggle(value)}
        >
          <X class="size-3" />
        </span>
      </span>
    {:else}
      <span class="py-0.5 text-ink-faint">{placeholder}</span>
    {/each}

    <ChevronDown class="ml-auto size-4 shrink-0 text-ink-faint" />
  {/snippet}

  {#if custom}
    <li>
      <button type="button" class="option" onclick={() => add(custom)}>
        <Plus class="size-3.5 shrink-0 text-ink-faint" />
        <span class="min-w-0 flex-1 truncate">{custom}</span>
        <span class="shrink-0 text-[11px] text-ink-faint">add</span>
      </button>
    </li>
  {/if}

  {#each shown as one (one.value)}
    <li>
      <button type="button" class="option" onclick={() => toggle(one.value)}>
        <span class="min-w-0 flex-1 truncate">{one.label}</span>
        {#if values.includes(one.value)}
          <Check class="size-3.5 shrink-0 text-accent" />
        {/if}
      </button>
    </li>
  {:else}
    {#if !custom}
      <li class="px-3 py-4 text-center text-xs text-ink-faint">No match</li>
    {/if}
  {/each}
</Dropdown>
