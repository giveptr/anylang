import { within } from '$lib/scope';

export type Branch<T> = {
  at: string;
  label: string;
  many: number;
  own: T[];
  kids: Branch<T>[];
};

type Growing<T> = {
  at: string;
  label: string;
  many: number;
  own: T[];
  kids: Map<string, Growing<T>>;
};

type Order<T> = (one: Branch<T>, other: Branch<T>) => number;

const fresh = <T>(at: string, label: string): Growing<T> => ({
  at,
  label,
  many: 0,
  own: [],
  kids: new Map(),
});

const alone = <T>(node: Growing<T>) =>
  node.kids.size === 1 && node.own.length === 0;

const only = <T>(node: Growing<T>) =>
  node.kids.values().next().value as Growing<T>;

const hollow = <T>(node: Growing<T>) =>
  alone(node) && only(node).own.length === 0;

export function branches<T>(
  these: T[],
  spotOf: (one: T) => string,
  order: Order<T>,
): Branch<T>[] {
  const root = fresh<T>('', '');

  for (const one of these) {
    let node = root;
    const spot = spotOf(one);

    for (const step of spot ? spot.split('/') : []) {
      const at = node.at ? `${node.at}/${step}` : step;
      let kid = node.kids.get(step);

      if (!kid) {
        kid = fresh<T>(at, step);
        node.kids.set(step, kid);
      }

      kid.many += 1;
      node = kid;
    }

    node.own.push(one);
  }

  let top = root;
  while (hollow(top)) top = only(top);

  return grown(top, order);
}

function grown<T>(node: Growing<T>, order: Order<T>): Branch<T>[] {
  return [...node.kids.values()].map((kid) => squeezed(kid, order)).sort(order);
}

function squeezed<T>(node: Growing<T>, order: Order<T>): Branch<T> {
  let label = node.label;
  let held = node;

  while (alone(held)) {
    held = only(held);
    label = `${label}/${held.label}`;
  }

  return {
    at: held.at,
    label,
    many: held.many,
    own: held.own,
    kids: grown(held, order),
  };
}

export type Found<T> = { shown: Branch<T>[]; holds: Set<string> };

export function sifted<T>(
  these: Branch<T>[],
  needle: string,
  named: (one: T) => string,
): Found<T> {
  const holds = new Set<string>();

  const walk = (list: Branch<T>[]): Branch<T>[] => {
    const out: Branch<T>[] = [];

    for (const one of list) {
      if (one.label.toLowerCase().includes(needle)) {
        out.push(one);
        continue;
      }

      const own = one.own.filter((held) =>
        named(held).toLowerCase().includes(needle),
      );
      const kids = walk(one.kids);

      if (!own.length && !kids.length) continue;

      holds.add(one.at);
      out.push({ ...one, own, kids });
    }

    return out;
  };

  return { shown: walk(these), holds };
}

export function chainTo<T>(these: Branch<T>[], spot: string): Branch<T>[] {
  const out: Branch<T>[] = [];
  if (!spot) return out;

  let list = these;

  for (;;) {
    const one = list.find(
      (held) => within(held.at, spot) || within(spot, held.at),
    );
    if (!one) return out;

    out.push(one);
    if (within(spot, one.at)) return out;

    list = one.kids;
  }
}
