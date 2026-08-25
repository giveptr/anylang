<script lang="ts">
  import { pieces } from '$lib/format';

  let {
    text,
    marks,
    now = false,
  }: { text: string; marks: RegExp[]; now?: boolean } = $props();

  const parts = $derived(pieces(text, marks));
  const shade = $derived(now ? 'bg-highlight-strong' : 'bg-highlight');
</script>

{#each parts as part, at (at)}{#if part.hit}<mark class={shade}
      >{part.text}</mark
    >{:else}{part.text}{/if}{/each}
