<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { commands, type Settings } from '$lib/bindings';
  import { caught } from '$lib/save';
  import { firstModel, presetOf, presets } from '$lib/providers';
  import FileField from '$lib/components/file-field.svelte';
  import TemperatureField, {
    WARM,
  } from '$lib/components/temperature-field.svelte';
  import Picker from '$lib/components/picker.svelte';
  import TextField from '$lib/components/text-field.svelte';

  let settings = $state<Settings | null>(null);
  let saved = $state('');
  let problem = $state('');
  let waiting: Settings | null = null;

  const options = presets.map((one) => ({ value: one.id, label: one.label }));
  let chosen = $state('');

  function choose(id: string) {
    const picked = presets.find((one) => one.id === id);
    if (!settings || !picked || id === chosen) return;

    chosen = id;
    settings.using = picked.kind;
    if (picked.url) settings.compatible.baseUrl = picked.url;

    if (picked.kind === 'compatible' && picked.firstModel)
      settings.compatible.model = picked.firstModel;
  }

  onMount(() => {
    caught(() => commands.loadSettings()).then((result) => {
      if (result.status === 'error') {
        problem = result.error;
        return;
      }
      const loaded = result.data;
      const preset = presetOf(loaded.using, loaded.compatible.baseUrl);

      loaded.gemini.model ||= firstModel('gemini');
      loaded.vertex.model ||= firstModel('vertex');
      loaded.claude.model ||= firstModel('claude');
      if (loaded.using === 'compatible')
        loaded.compatible.model ||= firstModel(preset);

      loaded.gemini.temperature ??= WARM;
      loaded.vertex.temperature ??= WARM;
      loaded.compatible.temperature ??= WARM;

      saved = JSON.stringify(loaded);
      settings = loaded;
      chosen = preset;
    });
  });

  $effect(() => {
    const snapshot = settings && $state.snapshot(settings);
    if (!snapshot) return;
    if (!snapshot.linesPerRequest || !snapshot.parallelRequests) return;

    const now = JSON.stringify(snapshot);
    if (now === saved) {
      waiting = null;
      return;
    }

    waiting = snapshot;
    const timer = setTimeout(async () => {
      waiting = null;
      const result = await caught(() => commands.saveSettings(snapshot));
      if (result.status === 'error') {
        problem = result.error;
      } else {
        saved = now;
        problem = '';
      }
    }, 400);

    return () => clearTimeout(timer);
  });

  onDestroy(() => {
    const held = waiting;
    if (held) void caught(() => commands.saveSettings(held));
  });
</script>

<div class="flex flex-col gap-5">
  {#if settings}
    <div class="flex flex-col gap-1.5">
      <span class="text-sm font-medium">Provider</span>
      <Picker value={chosen} onpick={choose} {options} searchable={false} />
    </div>

    {#if settings.using === 'gemini'}
      <TextField label="API key" secret bind:value={settings.gemini.apiKey} />
      <TextField
        label="Model name"
        bind:value={settings.gemini.model}
        placeholder="Type the exact model name"
      />
      <TemperatureField bind:value={settings.gemini.temperature} />
    {:else if settings.using === 'vertex'}
      <FileField
        label="Service account file"
        value={settings.vertex.credentials}
        filters={[{ name: 'JSON', extensions: ['json'] }]}
        onpick={(picked) => {
          if (settings) settings.vertex.credentials = picked;
        }}
      />
      <TextField
        label="Model name"
        bind:value={settings.vertex.model}
        placeholder="Type the exact model name"
      />
      <TemperatureField bind:value={settings.vertex.temperature} />
    {:else if settings.using === 'claude'}
      <TextField label="API key" secret bind:value={settings.claude.apiKey} />
      <TextField
        label="Model name"
        bind:value={settings.claude.model}
        placeholder="Type the exact model name"
      />
    {:else}
      {#if chosen === 'custom'}
        <TextField label="Base URL" bind:value={settings.compatible.baseUrl} />
      {/if}
      <TextField
        label="API key"
        secret
        bind:value={settings.compatible.apiKey}
      />
      <TextField
        label="Model name"
        bind:value={settings.compatible.model}
        placeholder="Type the exact model name"
      />
      <TemperatureField bind:value={settings.compatible.temperature} />
    {/if}

    <div class="grid grid-cols-2 gap-4 border-t border-line pt-5">
      <label class="flex flex-col gap-1.5">
        <span class="text-sm font-medium">Lines per request</span>
        <input
          class="box-input"
          type="number"
          min="1"
          bind:value={settings.linesPerRequest}
        />
      </label>
      <label class="flex flex-col gap-1.5">
        <span class="text-sm font-medium">Parallel requests</span>
        <input
          class="box-input"
          type="number"
          min="1"
          bind:value={settings.parallelRequests}
        />
      </label>
    </div>
  {/if}

  {#if problem}
    <p class="rounded-lg bg-alarm-wash p-3 text-sm text-alarm">{problem}</p>
  {/if}
</div>
