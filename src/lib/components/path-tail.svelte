<script lang="ts">
  let { path }: { path: string } = $props();

  const cut = $derived(path.lastIndexOf('/'));
  const split = $derived(cut > 0 && cut < path.length - 1);
  const under = $derived(split ? path.slice(0, cut) : '');
  const name = $derived(split ? path.slice(cut + 1) : path);
</script>

<span class="flex min-w-0 items-baseline"
  >{#if under}<span class="min-w-0 shrink-[9999] truncate">{under}</span><span
      class="shrink-0">/</span
    >{/if}<span dir="rtl" class="min-w-0 truncate">{name}</span></span
>
