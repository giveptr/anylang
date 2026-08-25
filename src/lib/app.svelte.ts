import { SvelteSet } from 'svelte/reactivity';
import type { Font, Preparing, Project, Tally } from '$lib/bindings';
import type { View } from '$lib/views';

export type Pane =
  'general' | 'source' | 'languages' | 'prompt' | 'fonts' | 'log';

export const app = $state({
  gameDir: '',
  survey: null as Tally | null,
  project: null as Project | null,
  piles: false,
  sources: [] as string[],
  faces: [] as Font[],
  preparing: null as Preparing | null,
  view: 'text' as View,
  pane: 'general' as Pane,
  running: false,
  busy: new SvelteSet<string>(),
  notice: null as { text: string; tone: 'bad' | 'good' } | null,
  working: false,
});

export function quiet() {
  app.notice = null;
}

export function alarm(text: string) {
  app.notice = { text, tone: 'bad' };
}

export function tell(text: string) {
  app.notice = { text, tone: 'good' };
}

export const language = () => app.project?.language?.trim() ?? '';

export const inProject = () => !app.preparing && app.survey !== null;
