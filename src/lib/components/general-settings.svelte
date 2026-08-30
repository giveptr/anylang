<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { commands, type Settings } from '$lib/bindings';
  import { caught } from '$lib/save';
  import { firstModel, presets } from '$lib/providers';
  import FileField from '$lib/components/file-field.svelte';
  import Picker from '$lib/components/picker.svelte';
  import TextField from '$lib/components/text-field.svelte';

  let settings = $state<Settings | null>(null);
  let saved = $state('');
  let problem = $state('');
  let waiting: Settings | null = null;

  const MODEL_DECIDES = 'Leave empty to let the model decide';

  const options = presets.map((one) => ({ value: one.id, label: one.label }));
  const chosen = $derived(settings?.preset ?? '');
  const endpoint = $derived(settings?.endpoints[chosen]);

  function remember(held: Settings, id: string) {
    held.endpoints[id] ??= {
      baseUrl: presets.find((one) => one.id === id)?.url ?? '',
      apiKey: '',
      model: firstModel(id),
      temperature: '',
    };
  }

  function choose(id: string) {
    const picked = presets.find((one) => one.id === id);
    if (!settings || !picked || id === chosen) return;

    settings.preset = id;
    settings.using = picked.kind;
    if (picked.kind === 'compatible') remember(settings, id);
  }

  onMount(() => {
    caught(() => commands.loadSettings()).then((result) => {
      if (result.status === 'error') {
        problem = result.error;
        return;
      }
      const loaded = result.data;
      const preset = (loaded.preset ||=
        loaded.using === 'compatible' ? 'custom' : loaded.using);

      loaded.gemini.model ||= firstModel('gemini');
      loaded.vertex.model ||= firstModel('vertex');
      loaded.claude.model ||= firstModel('claude');
      if (loaded.using === 'compatible') remember(loaded, preset);

      saved = JSON.stringify(loaded);
      settings = loaded;
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
      <TextField
        label="Temperature"
        bind:value={settings.gemini.temperature}
        placeholder={MODEL_DECIDES}
      />
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
      <TextField
        label="Temperature"
        bind:value={settings.vertex.temperature}
        placeholder={MODEL_DECIDES}
      />
    {:else if settings.using === 'claude'}
      <TextField label="API key" secret bind:value={settings.claude.apiKey} />
      <TextField
        label="Model name"
        bind:value={settings.claude.model}
        placeholder="Type the exact model name"
      />
    {:else if endpoint}
      {#if chosen === 'custom'}
        <TextField label="Base URL" bind:value={endpoint.baseUrl} />
      {/if}
      <TextField label="API key" secret bind:value={endpoint.apiKey} />
      <TextField
        label="Model name"
        bind:value={endpoint.model}
        placeholder="Type the exact model name"
      />
      <TextField
        label="Temperature"
        bind:value={endpoint.temperature}
        placeholder={MODEL_DECIDES}
      />
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
