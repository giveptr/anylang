import { convertFileSrc } from '@tauri-apps/api/core';
import { commands } from '$lib/bindings';
import { app } from '$lib/app.svelte';
import { saveProject } from '$lib/save';
import { filled, swapFor, withSwap, type Swap } from '$lib/swaps';

const loaded = $state<Record<string, string>>({});
const held: FontFace[] = [];
let age = 0;

function named(at: string) {
  return `face-${at.replace(/[^a-zA-Z0-9]/g, '-')}`;
}

export function drawn(at: string) {
  return !!loaded[at];
}

export function styled(at: string, family = '') {
  const wanted = (loaded[at] || family)
    .replace(/\p{Cc}/gu, '')
    .replace(/["\\]/g, '\\$&');

  return wanted ? `font-family: "${wanted}", "notdef"` : '';
}

export async function show(at: string) {
  if (!at || loaded[at] !== undefined || typeof FontFace === 'undefined')
    return;

  const mine = age;
  const stale = () => mine !== age;

  loaded[at] = '';

  try {
    const allowed = await commands.fontShown(at);
    if (allowed.status !== 'ok' || stale()) return;

    const family = named(at);
    const face = new FontFace(family, `url("${convertFileSrc(at)}")`);
    await face.load();
    if (stale()) return;

    document.fonts.add(face);
    held.push(face);
    loaded[at] = family;
  } catch {
    if (!stale()) loaded[at] = '';
  }
}

export function forgetFonts() {
  age += 1;
  for (const face of held) document.fonts.delete(face);
  held.length = 0;

  for (const at of Object.keys(loaded)) delete loaded[at];
}

const swaps = () => app.project?.fonts?.swaps ?? [];

export const sent = () => filled(swaps());

export function sentTo(name: string) {
  return swapFor(swaps(), name);
}

function send(swaps: Swap[]) {
  if (!app.project) return;

  app.project.fonts = { ...app.project.fonts, swaps };

  void saveProject();
}

export function sendTo(name: string, to: string) {
  send(withSwap(swaps(), name, to));
}

export function sendAllTo(to: string) {
  send(to ? app.faces.map((one) => ({ from: one.name, to })) : []);
}
