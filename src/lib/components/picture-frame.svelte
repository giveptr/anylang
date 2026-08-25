<script lang="ts">
  type Props = {
    src: string;
    name: string;
    ring?: string;
    onzoom: () => void;
  };

  let { src, name, ring = 'ring-1 ring-edge', onzoom }: Props = $props();
</script>

<button
  class="flex h-52 w-full cursor-zoom-in items-center justify-center overflow-hidden rounded-md checkers {ring}"
  aria-label="See this picture up close"
  onclick={onzoom}
>
  <img
    {src}
    alt={name}
    draggable="false"
    class="max-h-full max-w-full object-contain"
  />
</button>
