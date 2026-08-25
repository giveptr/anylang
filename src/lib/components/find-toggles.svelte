<script lang="ts">
  import { CaseSensitive, Regex, WholeWord } from '@lucide/svelte';
  import type { Seeking } from '$lib/bindings';
  import Hint from '$lib/components/hint.svelte';

  let { how = $bindable(), onpick }: { how: Seeking; onpick?: () => void } =
    $props();

  const switches = [
    { key: 'cased', label: 'Match case', Icon: CaseSensitive },
    { key: 'whole', label: 'Whole word', Icon: WholeWord },
    { key: 'regex', label: 'Use a pattern', Icon: Regex },
  ] as const;
</script>

<div class="flex shrink-0 items-center gap-0.5">
  {#each switches as { key, label, Icon } (key)}
    <Hint text={label}>
      <button
        type="button"
        aria-label={label}
        aria-pressed={how[key]}
        class="grid size-6 place-items-center rounded transition-colors {how[
          key
        ]
          ? 'bg-selected text-on-selected'
          : 'text-ink-faint hover:bg-sunken hover:text-ink'}"
        onmousedown={(event) => event.preventDefault()}
        onclick={() => {
          how = { ...how, [key]: !how[key] };
          onpick?.();
        }}
      >
        <Icon class="size-3.5" />
      </button>
    </Hint>
  {/each}
</div>
