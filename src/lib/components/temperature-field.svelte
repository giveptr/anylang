<script lang="ts" module>
  export const WARM = 0.8;
</script>

<script lang="ts">
  const COOLEST = 0;
  const HOTTEST = 2;
  const STEP = 0.1;

  let { value = $bindable() }: { value: number | null | undefined } = $props();

  const shown = $derived(
    Math.min(
      HOTTEST,
      Math.max(COOLEST, Number.isFinite(value) ? Number(value) : WARM),
    ),
  );
</script>

<label class="flex flex-col gap-1.5">
  <span class="flex items-baseline justify-between text-sm font-medium">
    Temperature
    <span class="font-normal text-ink-soft tabular-nums"
      >{shown.toFixed(1)}</span
    >
  </span>
  <input
    class="w-full cursor-pointer accent-accent"
    type="range"
    min={COOLEST}
    max={HOTTEST}
    step={STEP}
    value={shown}
    oninput={(event) => (value = event.currentTarget.valueAsNumber)}
  />
</label>
