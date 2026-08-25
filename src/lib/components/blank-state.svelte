<script lang="ts">
  import type { FileText } from '@lucide/svelte';

  type Props = {
    Icon: typeof FileText;
    said: string;
  };

  let { Icon, said }: Props = $props();
</script>

<Icon class="size-8 text-ink-faint" strokeWidth={1.5} />
<p class="text-sm text-ink-faint">{said}</p>
