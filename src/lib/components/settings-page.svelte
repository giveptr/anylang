<script lang="ts">
  import { app, type Pane } from '$lib/app.svelte';
  import { log, saw } from '$lib/log.svelte';
  import PromptForm from '$lib/components/prompt-form.svelte';
  import GeneralSettings from '$lib/components/general-settings.svelte';
  import LanguageSettings from '$lib/components/language-settings.svelte';
  import SourceSettings from '$lib/components/source-settings.svelte';
  import FontSettings from '$lib/components/font-settings.svelte';
  import LogView from '$lib/components/log-view.svelte';
  import { reads } from '$lib/sources';

  const every: { key: Pane; label: string }[] = [
    { key: 'general', label: 'General' },
    { key: 'source', label: 'Source' },
    { key: 'languages', label: 'Languages' },
    { key: 'prompt', label: 'Prompt' },
    { key: 'fonts', label: 'Font override' },
    { key: 'log', label: 'Logs' },
  ];

  const offered = $derived({
    source: !!app.project && reads(app.project, app.sources).length > 1,
    fonts: app.faces.length > 0,
  });

  const tabs = $derived(
    every.filter((one) =>
      one.key === 'source' || one.key === 'fonts' ? offered[one.key] : true,
    ),
  );

  const shown = $derived(
    tabs.some((one) => one.key === app.pane) ? app.pane : 'general',
  );

  $effect(() => {
    if (shown === 'log' && log.unseen) saw();
  });
</script>

<div class="grid h-full min-h-0 grid-cols-[13rem_minmax(0,1fr)] gap-px bg-line">
  <nav class="flex min-h-0 flex-col gap-0.5 bg-surface p-3">
    {#each tabs as tab (tab.key)}
      <button
        class="flex items-center gap-2 rounded-md px-3 py-2 text-left text-sm transition-colors {shown ===
        tab.key
          ? 'bg-selected font-medium text-on-selected'
          : 'text-ink-soft hover:bg-sunken hover:text-ink'}"
        onclick={() => (app.pane = tab.key)}
      >
        {tab.label}
        {#if tab.key === 'log' && log.unseen}
          <span class="size-1.5 rounded-full bg-close"></span>
        {/if}
      </button>
    {/each}
  </nav>

  <div class="min-h-0 overflow-auto bg-surface p-6">
    {#if shown === 'log'}
      <LogView />
    {:else if shown === 'fonts'}
      <FontSettings />
    {:else}
      <div class="mx-auto {shown === 'prompt' ? 'max-w-5xl' : 'max-w-lg'}">
        {#if shown === 'general'}
          <GeneralSettings />
        {:else if app.project && shown === 'source'}
          <SourceSettings project={app.project} />
        {:else if app.project && shown === 'languages'}
          <LanguageSettings project={app.project} />
        {:else if app.project && shown === 'prompt'}
          <PromptForm project={app.project} />
        {/if}
      </div>
    {/if}
  </div>
</div>
