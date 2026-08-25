<script lang="ts">
  import { FolderUp } from '@lucide/svelte';
  import { onMount } from 'svelte';
  import { getCurrentWebview } from '@tauri-apps/api/webview';
  import { open } from '@tauri-apps/plugin-dialog';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { GITHUB, about } from '$lib/about.svelte';
  import { app } from '$lib/app.svelte';

  let { onpick }: { onpick: (dir: string) => void } = $props();

  let hovering = $state(false);

  onMount(() => {
    const stop = getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === 'over') hovering = true;
      else if (event.payload.type === 'drop') {
        hovering = false;
        const [first] = event.payload.paths;
        if (first) onpick(first);
      } else hovering = false;
    });

    return () => void stop.then((remove) => remove());
  });

  async function browse() {
    const chosen = await open({ directory: true, multiple: false });
    if (typeof chosen === 'string') onpick(chosen);
  }
</script>

<div class="flex h-full flex-col px-6 pb-6">
  <div class="grid flex-1 place-items-center">
    <div class="flex w-full max-w-md flex-col items-center">
      <button
        type="button"
        onclick={browse}
        class="flex w-full flex-col items-center gap-5 rounded-2xl border-2 border-dashed px-10 py-16 text-center transition-colors {hovering
          ? 'border-accent bg-accent-wash'
          : 'border-line bg-transparent'}"
      >
        <FolderUp
          class="size-8 transition-colors {hovering
            ? 'text-accent'
            : 'text-ink-faint'}"
        />

        <span class="flex flex-col gap-1.5">
          <span class="text-lg font-medium">
            {hovering ? 'Let go to open it' : 'Drop a game folder'}
          </span>
          <span class="text-sm text-ink-soft"
            >Works with Ren'Py, RPG Maker, Wolf RPG and Unity games</span
          >
        </span>

        <span class="text-sm font-medium text-accent">or click to browse</span>
      </button>

      <p
        role="alert"
        class="mt-3 min-h-5 w-full text-center text-sm {app.notice?.tone ===
        'bad'
          ? 'text-alarm'
          : 'text-done'}"
      >
        {app.notice?.text ?? ''}
      </p>
    </div>
  </div>

  <div
    class="flex shrink-0 items-center justify-center gap-2 text-xs text-ink-faint"
  >
    <span class="tabular-nums">v{about.version}</span>

    <span aria-hidden="true">·</span>
    <button
      type="button"
      class="rounded transition-colors hover:text-ink hover:underline"
      onclick={() => openUrl(GITHUB)}
    >
      GitHub
    </button>
  </div>
</div>
