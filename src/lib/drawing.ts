export type Wanted = { key: string; most: number };

let queue: Wanted[] = [];
let age = 0;
const flying = new Set<string>();
const done = new Set<string>();

export const aged = () => age;

export const named = (one: Wanted) => `${one.key}|${one.most}`;

export function waiting(each: Wanted[]) {
  queue = each.filter((one) => {
    const at = named(one);

    return !done.has(at) && !flying.has(at);
  });
}

export function taken(atOnce: number) {
  if (flying.size >= atOnce) return undefined;

  const one = queue.shift();
  if (one) flying.add(named(one));

  return one;
}

export function answered(one: Wanted) {
  done.add(named(one));
}

export function dropped(one: Wanted) {
  flying.delete(named(one));
}

export function forgot(at: string) {
  done.delete(at);
}

export function cleared() {
  queue = [];
  done.clear();
  flying.clear();
  age += 1;
}
