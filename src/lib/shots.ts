import type { Shot } from '$lib/bindings';

export function spotOf(one: Shot) {
  const at = one.at ?? '';
  if (!at.startsWith(`${one.holder}/`)) return one.holder;

  const cut = at.lastIndexOf('/');

  return cut > 0 ? at.slice(0, cut) : one.holder;
}

export function sheetsIn(these: Shot[]) {
  const out: Record<string, number> = {};

  for (const one of these) {
    if (one.atlas) out[one.atlas] = (out[one.atlas] ?? 0) + 1;
  }

  return Object.entries(out).sort((a, b) => a[0].localeCompare(b[0]));
}
