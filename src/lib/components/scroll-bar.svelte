<script lang="ts">
  const THINNEST = 24;

  let {
    of,
    tall,
    view,
    at,
    onmove,
  }: {
    of: string;
    tall: number;
    view: number;
    at: number;
    onmove: (top: number) => void;
  } = $props();

  let track = $state<HTMLElement | null>(null);
  let held = $state(false);

  const room = $derived(Math.max(0, tall - view));
  const needed = $derived(room > 0 && view > 0);
  const thumb = $derived(Math.round(Math.max(THINNEST, view * (view / tall))));
  const top = $derived(room > 0 ? Math.round((at / room) * (view - thumb)) : 0);
  const along = $derived(room > 0 ? Math.round((at / room) * 100) : 0);

  function sendTo(pointer: number) {
    if (!track) return;

    const box = track.getBoundingClientRect();
    const wanted = pointer - box.top - thumb / 2;
    const reach = box.height - thumb;

    onmove(
      reach > 0 ? (Math.min(Math.max(wanted, 0), reach) / reach) * room : 0,
    );
  }

  function drag(event: PointerEvent) {
    if (held) sendTo(event.clientY);
  }

  function grab(event: PointerEvent) {
    event.preventDefault();
    held = true;
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    sendTo(event.clientY);
  }
</script>

{#if needed}
  <div
    bind:this={track}
    role="scrollbar"
    tabindex="-1"
    aria-orientation="vertical"
    aria-controls={of}
    aria-valuenow={along}
    onpointerdown={grab}
    onpointermove={drag}
    onpointerup={() => (held = false)}
    onpointercancel={() => (held = false)}
    class="absolute top-0 right-0 z-20 w-2.5 touch-none opacity-0 transition-opacity group-hover/scroll:opacity-100 {held
      ? 'opacity-100'
      : ''}"
    style="height: {view}px"
  >
    <div
      class="absolute right-0.75 left-0.75 rounded-full transition-colors {held
        ? 'bg-ink-faint'
        : 'bg-edge hover:bg-ink-faint'}"
      style="height: {thumb}px; transform: translateY({top}px)"
    ></div>
  </div>
{/if}
