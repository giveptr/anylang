<script lang="ts">
  import { type Project } from '$lib/bindings';
  import { languages } from '$lib/languages';
  import Picker from '$lib/components/picker.svelte';

  let { project }: { project: Project } = $props();

  const choices = languages.map((name) => ({ value: name, label: name }));
</script>

<div class="grid grid-cols-2 gap-4">
  <div class="flex flex-col gap-1.5">
    <span class="text-sm font-medium">Translate from</span>
    <Picker
      bind:value={project.sourceLanguage}
      options={choices}
      placeholder="The game's language"
    />
  </div>

  <div class="flex flex-col gap-1.5">
    <span class="text-sm font-medium">Into</span>
    <Picker
      bind:value={project.language}
      options={choices}
      placeholder="Your language"
    />
  </div>
</div>
