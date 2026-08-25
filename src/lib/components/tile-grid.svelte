<script lang="ts" generics="T">
  import type { Snippet } from 'svelte';

  import ScrollBar from '$lib/components/scroll-bar.svelte';

  type Props = {
    items: T[];
    narrowest: number;
    labels: number;
    named: (one: T) => string;
    tile: Snippet<[T]>;
    onscreen: (cut: T[]) => void;
    chosen: T | null;
    onchoose: (one: T) => void;
  };

  const SPARE = 2;
  const AROUND = 24;
  const APART = 4;

  let {
    items,
    narrowest,
    labels,
    named,
    tile,
    onscreen,
    chosen,
    onchoose,
  }: Props = $props();

  let box = $state<HTMLDivElement | null>(null);
  let scrolled = $state(0);
  let tall = $state(0);
  let wide = $state(0);

  const columns = $derived(
    Math.max(1, Math.floor((wide || narrowest) / narrowest)),
  );
  const across = $derived(
    Math.max(
      1,
      Math.floor(
        ((wide || narrowest) - AROUND - (columns - 1) * APART) / columns,
      ),
    ),
  );
  const line = $derived(across + labels);

  const rows = $derived(Math.ceil(items.length / columns));
  const inside = $derived(scrolled - AROUND / 2);
  const first = $derived(
    Math.max(0, Math.floor(inside / line) - SPARE) * columns,
  );
  const past = $derived(
    Math.min(
      items.length,
      (Math.ceil((inside + (tall || line)) / line) + SPARE) * columns,
    ),
  );

  const cut = $derived(items.slice(first, past));
  const long = $derived(rows * line + AROUND - APART);
  const above = $derived(Math.floor(first / columns) * line);
  const below = $derived(
    Math.max(0, rows * line - above - Math.ceil(cut.length / columns) * line),
  );

  $effect(() => {
    onscreen(cut);
  });

  export function take() {
    box?.focus();
  }

  function landing(key: string, at: number, apart: Record<string, number>) {
    if (key === 'Home') return 0;
    if (key === 'End') return items.length - 1;

    const move = apart[key];
    if (move === undefined) return null;

    return at < 0 ? 0 : at + move;
  }

  export function step(event: KeyboardEvent) {
    if (items.length === 0) return;

    const at = chosen
      ? items.findIndex((one) => named(one) === named(chosen))
      : -1;
    const down = Math.max(1, Math.floor((tall || line) / line)) * columns;
    const wanted = landing(event.key, at, {
      ArrowRight: 1,
      ArrowLeft: -1,
      ArrowDown: columns,
      ArrowUp: -columns,
      PageDown: down,
      PageUp: -down,
    });

    if (wanted === null) return;

    event.preventDefault();

    const next = Math.min(Math.max(wanted, 0), items.length - 1);
    onchoose(items[next]);
    seen(next);
  }

  function seen(which: number) {
    if (!box) return;

    const top = AROUND / 2 + Math.floor(which / columns) * line;
    const view = box.clientHeight;

    if (top < box.scrollTop) box.scrollTop = top;
    else if (top + line > box.scrollTop + view)
      box.scrollTop = top + line - view;
  }

  export function reset() {
    scrolled = 0;
    if (box) box.scrollTop = 0;
  }
</script>

<div class="group/scroll relative min-h-0 flex-1">
  <ScrollBar
    of="tile-grid"
    tall={long}
    view={tall}
    at={scrolled}
    onmove={(top) => box?.scrollTo({ top })}
  />

  <div
    bind:this={box}
    id="tile-grid"
    role="grid"
    tabindex="-1"
    class="h-full overflow-auto outline-none"
    style="padding: {AROUND / 2}px"
    bind:clientWidth={wide}
    bind:clientHeight={tall}
    onscroll={(event) => (scrolled = event.currentTarget.scrollTop)}
    onkeydown={step}
  >
    <div style="height: {above}px"></div>
    <div
      class="grid"
      style="gap: {APART}px; grid-template-columns: repeat({columns}, minmax(0, 1fr)); grid-auto-rows: {line -
        APART}px"
    >
      {#each cut as one (named(one))}
        {@render tile(one)}
      {/each}
    </div>
    <div style="height: {below}px"></div>
  </div>
</div>
