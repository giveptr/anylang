<script lang="ts">
  import { Moon, Sun } from '@lucide/svelte';
  import { flip, theme } from '$lib/theme.svelte';
</script>

<button
  type="button"
  class="chrome-button h-full w-11"
  aria-label={theme.dark ? 'Use the light theme' : 'Use the dark theme'}
  onclick={flip}
>
  {#if theme.dark}
    <Sun class="size-4" />
  {:else}
    <Moon class="size-4" />
  {/if}
</button>
