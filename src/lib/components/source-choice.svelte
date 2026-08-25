<script lang="ts">
  import { paths, sourceLabel } from '$lib/sources';
  import { READ_FROM } from '$lib/wording';

  let {
    sources,
    onread,
  }: { sources: string[]; onread: (folder: string) => void } = $props();

  const choices = $derived(paths(sources));
</script>

<div class="grid h-full place-items-center px-6 pb-12">
  <div class="flex w-full max-w-md flex-col gap-3">
    <span class="text-lg font-medium">{READ_FROM}</span>

    <ol
      class="flex flex-col overflow-hidden rounded-lg bg-surface ring-1 ring-line"
    >
      {#each choices as one (one)}
        <li>
          <button type="button" class="option" onclick={() => onread(one)}>
            {sourceLabel(one)}
          </button>
        </li>
      {/each}
    </ol>
  </div>
</div>
