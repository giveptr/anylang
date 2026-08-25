<script lang="ts">
  import { X } from '@lucide/svelte';
  import { app, quiet } from '$lib/app.svelte';
</script>

{#if app.notice}
  {@const notice = app.notice}
  <div
    role="alert"
    class="relative flex shrink-0 flex-wrap items-center justify-center gap-x-3 gap-y-1 border-b border-line bg-surface px-12 py-2.5 text-center text-sm"
  >
    <p
      class="min-w-0 wrap-anywhere {notice.tone === 'bad'
        ? 'text-alarm'
        : 'text-done'}"
    >
      {notice.text}
    </p>

    <button
      class="absolute top-1.5 right-3 icon-button size-7"
      aria-label="Close this message"
      onclick={quiet}
    >
      <X class="size-4" />
    </button>
  </div>
{/if}
