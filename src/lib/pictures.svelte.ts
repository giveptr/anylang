import { SvelteMap } from 'svelte/reactivity';
import { about } from '$lib/about.svelte';
import { commands, type Pictures, type Shot } from '$lib/bindings';
import { alarm, app } from '$lib/app.svelte';
import {
  aged,
  answered,
  cleared,
  dropped,
  forgot,
  named,
  taken,
  waiting,
  type Wanted,
} from '$lib/drawing';
import { filled, swapFor, withSwap } from '$lib/swaps';
import { caught, saveProject } from '$lib/save';

export const TILE = 160;
export const FULL = 0;

export const gallery = $state({
  shots: [] as Shot[],
  loading: false,
  game: '',
});

type Kept<T> = {
  get: (at: string) => T | undefined;
  has: (at: string) => boolean;
  set: (at: string, one: T) => void;
  keeping: (each: string[]) => void;
  clear: () => void;
};

type Budget<T> = {
  sizeOf: (one: T) => number;
  roomy: number;
  many?: number;
  gone?: (at: string) => void;
};

function budgeted<T>({
  sizeOf,
  roomy,
  many = Infinity,
  gone,
}: Budget<T>): Kept<T> {
  const held = new SvelteMap<string, T>();
  const order: string[] = [];
  let needed: Record<string, boolean> = {};
  let room = 0;

  const over = () => order.length > many || room > roomy;

  function trim() {
    for (let at = 0; at < order.length && over();) {
      const one = order[at];
      if (needed[one]) {
        at += 1;
        continue;
      }

      const was = held.get(one);
      order.splice(at, 1);
      held.delete(one);
      if (was !== undefined) room -= sizeOf(was);
      gone?.(one);
    }
  }

  return {
    get: (at) => held.get(at),
    has: (at) => held.has(at),
    set(at, one) {
      const was = held.get(at);
      if (was !== undefined) {
        room -= sizeOf(was);
        order.splice(order.indexOf(at), 1);
      }

      order.push(at);
      held.set(at, one);
      room += sizeOf(one);
      trim();
    },
    keeping(each) {
      needed = {};
      for (const one of each) needed[one] = true;
    },
    clear() {
      held.clear();
      order.length = 0;
      needed = {};
      room = 0;
    },
  };
}

type Fetched = { ok: true; source: string } | { ok: false; error: string };

const weighs = (one: Fetched) =>
  one.ok ? one.source.length : one.error.length;

const drawn = budgeted<Fetched>({
  sizeOf: weighs,
  roomy: 64 * 1024 * 1024,
  many: 240,
  gone: forgot,
});

export function shown(key: string, most: number) {
  const held = drawn.get(named({ key, most }));

  return held?.ok ? held.source : '';
}

export function whyBlank(key: string, most: number) {
  const held = drawn.get(named({ key, most }));

  return held && !held.ok ? held.error : '';
}

export function want(each: Wanted[]) {
  if (!app.gameDir) return;

  drawn.keeping(each.map(named));
  waiting(each);
  fill();
}

function fill() {
  const many = about.atOnce;

  for (let one = taken(many); one; one = taken(many)) void pull(one);
}

async function pull(one: Wanted) {
  const dir = app.gameDir;
  const when = aged();
  const stale = () => dir !== app.gameDir || when !== aged();

  try {
    const held = await caught(() =>
      commands.pictureShown(dir, one.key, one.most),
    );
    if (stale()) return;

    answered(one);
    drawn.set(
      named(one),
      held.status === 'ok'
        ? { ok: true, source: drawnFrom(held.data) }
        : { ok: false, error: held.error },
    );
  } finally {
    if (!stale()) dropped(one);
    fill();
  }
}

const drawnFrom = ({ body, mime }: { body: string; mime: string }) =>
  `data:${mime};base64,${body}`;

type Replacement =
  | { ok: true; source: string; wide: number; high: number }
  | { ok: false; error: string };

const replacements = budgeted<Replacement>({
  sizeOf: weighs,
  roomy: 32 * 1024 * 1024,
});
let fetching: Record<string, boolean> = {};

export function replacementShown(at: string) {
  return replacements.get(at) ?? null;
}

export async function showReplacement(at: string) {
  if (!at) return;

  replacements.keeping([at]);
  if (fetching[at] || replacements.has(at)) return;
  fetching[at] = true;

  const when = aged();
  try {
    const done = await caught(() => commands.replacementShown(at));
    if (when !== aged()) return;

    if (done.status === 'ok') {
      replacements.set(at, {
        ok: true,
        source: drawnFrom(done.data),
        wide: done.data.wide,
        high: done.data.high,
      });
    } else {
      replacements.set(at, { ok: false, error: done.error });
    }
  } finally {
    delete fetching[at];
  }
}

export function forgetPictures() {
  drawn.clear();
  cleared();
  replacements.clear();
  fetching = {};
  gallery.shots = [];
  gallery.game = '';
}

export async function load() {
  if (!app.gameDir || gallery.loading || gallery.game === app.gameDir) return;

  const dir = app.gameDir;
  gallery.loading = true;
  try {
    const found = await caught(() => commands.pictures(dir));
    if (dir !== app.gameDir) return;

    if (found.status !== 'ok') {
      alarm(found.error);
      return;
    }

    gallery.shots = found.data;
    gallery.game = dir;
  } finally {
    gallery.loading = false;
  }
}

const swaps = () => app.project?.pictures?.swaps ?? [];

const marks = () => app.project?.pictures?.marked ?? [];

const starred = $derived.by(() => {
  const keys: Record<string, boolean> = {};
  for (const one of marks()) keys[one] = true;

  return keys;
});

export const marked = (key: string) => starred[key] ?? false;

function saved(held: Partial<Pictures>) {
  if (!app.project) return;

  app.project.pictures = { ...app.project.pictures, ...held };

  void saveProject();
}

export function toggleMark(key: string) {
  const held = marks();

  saved({
    marked: held.includes(key)
      ? held.filter((one) => one !== key)
      : [...held, key],
  });
}

export function swappedTo(key: string) {
  return swapFor(swaps(), key);
}

export function swapTo(key: string, to: string) {
  saved({ swaps: withSwap(swaps(), key, to) });
}

export const swapped = () => filled(swaps());
