<script lang="ts">
  import type { Project } from '$lib/bindings';
  import { app } from '$lib/app.svelte';
  import { reads, setShipped, shippedOf } from '$lib/sources';
  import { READ_FROM } from '$lib/wording';
  import Picker from '$lib/components/picker.svelte';

  let { project }: { project: Project } = $props();

  const shipped = $derived(shippedOf(project));
  const options = $derived(reads(project, app.sources));

  function choose(folder: string) {
    setShipped(project, folder);
  }
</script>

<div class="flex flex-col gap-1.5">
  <span class="text-sm font-medium">{READ_FROM}</span>
  <Picker value={shipped} {options} searchable={false} onpick={choose} />
</div>
