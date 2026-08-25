const ROOT = '';

export const WHOLE: string[] = [ROOT];

export function within(scope: string, key: string) {
  return key === scope || key.startsWith(`${scope}/`);
}

export function climb<T>(
  key: string,
  look: (at: string) => T | undefined,
): T | undefined {
  let at = key;

  for (;;) {
    const found = look(at);
    if (found !== undefined) return found;

    const cut = at.lastIndexOf('/');
    if (cut <= 0) return undefined;

    at = at.slice(0, cut);
  }
}

export function shared(paths: string[]): string {
  const [first] = paths;
  if (!first || !first.includes('/')) return '';

  const head = `${first.split('/')[0]}/`;

  return paths.every((one) => one.startsWith(head) && one.length > head.length)
    ? head
    : '';
}

export const shortened = (path: string, head: string) =>
  head && path.startsWith(head) ? path.slice(head.length) : path;
