<script lang="ts">
  import '../app.css';
  import { onMount } from 'svelte';
  import {
    ChevronDown,
    FolderOutput,
    RefreshCw,
    Trash2,
    Undo2,
  } from '@lucide/svelte';
  import { about, learn } from '$lib/about.svelte';
  import { app, inProject } from '$lib/app.svelte';
  import { RESTORE } from '$lib/wording';
  import { views } from '$lib/views';
  import { log } from '$lib/log.svelte';
  import { frame, reveal } from '$lib/frame.svelte';
  import { fileName } from '$lib/format';
  import {
    closeProject,
    deleteProject,
    locked,
    rereadGame,
    revertGame,
  } from '$lib/project';
  import ArmedButton from '$lib/components/armed-button.svelte';
  import ContextMenu from '$lib/components/context-menu.svelte';
  import MenuItem from '$lib/components/menu-item.svelte';
  import ThemeSwitch from '$lib/components/theme-switch.svelte';
  import WindowControls from '$lib/components/window-controls.svelte';

  let { children } = $props();

  let menu = $state<ReturnType<typeof ContextMenu> | null>(null);
  let wipe = $state<ReturnType<typeof ArmedButton> | null>(null);

  void learn();

  onMount(reveal);

  function block(event: MouseEvent) {
    const target = event.target;
    if (target instanceof Element && target.closest('input, textarea')) return;

    event.preventDefault();
  }

  function tabbing(event: KeyboardEvent) {
    if (event.key === 'Tab') document.documentElement.dataset.keys = '';
  }

  function pointing() {
    delete document.documentElement.dataset.keys;
  }
</script>

<svelte:window
  oncontextmenu={block}
  onkeydown={tabbing}
  onpointerdown={pointing}
/>

<div
  class="relative flex h-full flex-col overflow-hidden text-ink {frame.maximized
    ? ''
    : 'rounded-md'}"
>
  <header
    data-tauri-drag-region
    class="grid h-9 shrink-0 grid-cols-[1fr_auto_1fr] items-stretch bg-line select-none"
  >
    <div data-tauri-drag-region class="flex min-w-0 items-stretch">
      {#if inProject()}
        <nav class="flex items-stretch gap-1 pl-1">
          {#each views as tab (tab.view)}
            <button
              class="relative flex items-center gap-2 px-3 text-[13px] transition-colors {app.view ===
              tab.view
                ? 'font-medium text-ink'
                : 'text-ink-soft hover:text-ink'}"
              onclick={() => (app.view = tab.view)}
            >
              {tab.label}
              {#if tab.view === 'settings' && log.unseen}
                <span class="size-1.5 rounded-full bg-close"></span>
              {/if}
              {#if app.view === tab.view}
                <span
                  class="absolute inset-x-2 bottom-0 h-0.5 rounded-t-full bg-accent"
                ></span>
              {/if}
            </button>
          {/each}
        </nav>
      {:else}
        <span
          data-tauri-drag-region
          class="flex items-center truncate pl-4 text-[13px] font-medium text-ink-soft"
        >
          {about.name}
        </span>
      {/if}
    </div>

    <div
      data-tauri-drag-region
      class="flex min-w-0 items-center justify-center px-6"
    >
      {#if inProject()}
        <button
          class="flex min-w-0 items-center gap-1.5 rounded-md px-2 py-1 text-ink-faint transition-colors hover:bg-sunken hover:text-ink-soft"
          onclick={(event) => menu?.openBelow(event.currentTarget)}
        >
          <span class="truncate font-mono text-xs">
            {fileName(app.gameDir)}
          </span>
          <ChevronDown class="size-3 shrink-0" />
        </button>
      {:else if app.gameDir}
        <span
          data-tauri-drag-region
          class="truncate font-mono text-xs text-ink-faint"
        >
          {fileName(app.gameDir)}
        </span>
      {/if}
    </div>

    <div data-tauri-drag-region class="flex items-stretch justify-end">
      <ThemeSwitch />
      <WindowControls />
    </div>
  </header>

  <main class="min-h-0 flex-1 overflow-hidden bg-ground">
    {@render children()}
  </main>
</div>

<ContextMenu bind:this={menu} onclose={() => wipe?.disarm()}>
  <MenuItem label={RESTORE} Icon={Undo2} off={locked()} act={revertGame} />
  <MenuItem
    label="Read the game again"
    Icon={RefreshCw}
    off={locked()}
    act={rereadGame}
  />
  <MenuItem
    label="Close project"
    Icon={FolderOutput}
    off={locked()}
    act={closeProject}
  />

  <div class="h-px bg-line"></div>

  <ArmedButton
    bind:this={wipe}
    label="Delete project"
    armedLabel="Yes, delete it all"
    Icon={Trash2}
    disabled={locked()}
    warning="Deletes everything kept for this game. The game itself is untouched."
    onfire={deleteProject}
  />
</ContextMenu>
