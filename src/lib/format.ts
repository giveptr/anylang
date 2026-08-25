import type { Tally } from '$lib/bindings';

export const NOTHING: Tally = {
  files: 0,
  applied: 0,
  translated: 0,
  total: 0,
};

export function sumTallies(tallies: Tally[]): Tally {
  return tallies.reduce(
    (all, one) => ({
      files: all.files + one.files,
      applied: all.applied + one.applied,
      translated: all.translated + one.translated,
      total: all.total + one.total,
    }),
    { ...NOTHING },
  );
}

export const clockOf = (at: string) =>
  new Date(at).toLocaleTimeString(undefined, { hour12: false });

export const fileName = (path: string) =>
  path.split(/[/\\]/).filter(Boolean).pop() ?? path;

export function percent(done: number, total: number) {
  if (!total) return 0;
  if (done >= total) return 100;

  return Math.floor((done / total) * 100);
}

type Piece = { text: string; hit: boolean };

export function pieces(text: string, marks: RegExp[]): Piece[] {
  const spans: [number, number][] = [];

  for (const mark of marks) {
    mark.lastIndex = 0;
    for (;;) {
      const found = mark.exec(text);
      if (!found) break;

      if (found[0].length)
        spans.push([found.index, found.index + found[0].length]);
      else mark.lastIndex += 1;
    }
  }

  if (!spans.length) return [{ text, hit: false }];

  spans.sort((a, b) => a[0] - b[0] || a[1] - b[1]);

  const merged: [number, number][] = [];
  for (const [from, to] of spans) {
    const last = merged[merged.length - 1];
    if (last && from <= last[1]) last[1] = Math.max(last[1], to);
    else merged.push([from, to]);
  }

  const found: Piece[] = [];
  let at = 0;

  for (const [from, to] of merged) {
    if (from > at) found.push({ text: text.slice(at, from), hit: false });
    found.push({ text: text.slice(from, to), hit: true });
    at = to;
  }

  if (at < text.length) found.push({ text: text.slice(at), hit: false });

  return found;
}
