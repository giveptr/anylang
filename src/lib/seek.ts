import type { Row, Seeking } from '$lib/bindings';

const SHORTEST = 2;

export const longEnough = (needle: string) =>
  [...needle].length >= SHORTEST ||
  [...needle].some((one) => (one.codePointAt(0) ?? 0) > 127);

export const PLAIN: Seeking = { cased: false, whole: false, regex: false };

export function byLabel<T extends { label: string }>(
  options: T[],
  needle: string,
): T[] {
  if (!needle) return options;

  const wanted = needle.toLowerCase();
  return options.filter((one) => one.label.toLowerCase().includes(wanted));
}

export const same = (one: Seeking, other: Seeking) =>
  one.cased === other.cased &&
  one.whole === other.whole &&
  one.regex === other.regex;

export function holds(mark: RegExp, text: string) {
  mark.lastIndex = 0;
  return mark.test(text);
}

export const texts = (row: Row): string[] =>
  row.kind ? [row.label, row.kind] : [row.scope, row.label];

const WORD = '\\p{L}\\p{N}_';

export function pattern(needle: string, how: Seeking): RegExp | null {
  const said = needle.trim();
  if (!said) return null;

  const body = how.regex ? said : said.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const whole = `(?<![${WORD}])(?:${body})(?![${WORD}])`;

  try {
    return new RegExp(how.whole ? whole : body, how.cased ? 'gu' : 'giu');
  } catch {
    return null;
  }
}
