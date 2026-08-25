import { commands, type Exported, type Row, type Tally } from '$lib/bindings';
import { alarm, app } from '$lib/app.svelte';
import { NOTHING, sumTallies } from '$lib/format';
import { climb } from '$lib/scope';
import { caught } from '$lib/save';

export const rail = $state({
  rows: [] as Row[],
  chosen: { ...NOTHING },
  whole: { ...NOTHING },
  loading: true,
});

let index = $state<Record<string, Row>>({});

export function rowOf(key: string) {
  return climb(key, (at) => index[at]);
}

export function forgetRail() {
  rail.rows = [];
  rail.chosen = { ...NOTHING };
  rail.whole = { ...NOTHING };
  rail.loading = true;
  index = {};
}

function tallied(these: Row[]): Tally {
  return sumTallies(these.map((one) => one.tally));
}

function report() {
  if (!app.survey || rail.rows.length === 0) return;

  app.survey = { ...rail.chosen };
}

function retally() {
  rail.chosen = tallied(rail.rows.filter((one) => !one.excluded));
  rail.whole = tallied(rail.rows);

  report();
}

export function credited(key: string, filled: number, added: number) {
  const row = rowOf(key);
  if (!row) return;

  if (row.scope === key) {
    row.tally.translated = filled;
  } else {
    row.tally.translated += added;
  }

  retally();
}

export function edited(key: string, by: number) {
  const row = rowOf(key);
  if (!row) return;

  row.tally.translated += by;
  retally();
}

export async function loadRows(): Promise<string[] | null> {
  rail.loading = rail.rows.length === 0;

  let result;
  try {
    result = await caught(() => commands.listRows(app.gameDir));
  } finally {
    rail.loading = false;
  }

  if (result.status === 'error') {
    alarm(result.error);
    return null;
  }

  rail.rows = result.data.rows;
  rail.chosen = result.data.chosen;
  rail.whole = result.data.whole;

  const built: Record<string, Row> = {};
  for (const one of rail.rows) built[one.scope] ??= one;
  index = built;

  report();

  return [
    ...rail.rows.filter((one) => !one.excluded),
    ...rail.rows.filter((one) => one.excluded),
  ].map((one) => one.scope);
}

export function settle({ landed, gone }: Exported) {
  for (const [keys, by] of [
    [landed, 1],
    [gone, -1],
  ] as const) {
    for (const key of keys) {
      const row = rowOf(key);
      if (!row) continue;

      row.tally.applied = Math.min(
        Math.max(row.tally.applied + by, 0),
        row.tally.files,
      );
    }
  }

  retally();
}
