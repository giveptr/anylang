import { commands, type Opened } from '$lib/bindings';
import { alarm, app, language, quiet } from '$lib/app.svelte';
import { WHOLE } from '$lib/scope';
import { forgetLog, recall } from '$lib/log.svelte';
import { forgetFonts } from '$lib/fonts.svelte';
import { forgetPictures } from '$lib/pictures.svelte';
import { forgetRail, settle } from '$lib/rail.svelte';
import { caught, saveProject, tried, type Outcome } from '$lib/save';

export const halted = () => app.running || app.working;

export function locked() {
  return halted() || !app.project;
}

export async function alone<T>(
  work: () => Promise<Outcome<T>>,
  then?: (data: T) => void | Promise<void>,
) {
  app.working = true;

  try {
    const result = await tried(work);
    if (result.status === 'ok') await then?.(result.data);
  } finally {
    app.working = false;
  }
}

function unready() {
  const from = app.project?.sourceLanguage?.trim() ?? '';
  const into = language();

  if (!from || !into) return 'Pick both languages first.';

  if (from === into)
    return `Translate from and Into are both set to ${into}. Change one of them.`;

  return '';
}

export function ready() {
  const why = unready();
  if (why) askLanguages(why);

  return !why;
}

function askLanguages(why: string) {
  alarm(why);
  app.view = 'settings';
  app.pane = 'languages';
}

export async function stopScope(scopes: string[]) {
  await tried(() => commands.stopScope(scopes));
}

export async function resurvey() {
  if (!app.gameDir || !language()) return;

  const result = await caught(() => commands.survey(app.gameDir));
  if (!app.gameDir) return;

  if (result.status === 'error') alarm(result.error);
  else app.survey = result.data;
}

export async function revertGame() {
  if (locked()) return;

  await alone(() => commands.revertScope(app.gameDir, WHOLE), settle);
}

function forgetShown() {
  forgetPictures();
  forgetFonts();
}

export async function readGame(afresh: boolean) {
  if (!app.project) return false;

  const project = app.project;
  app.working = true;
  app.preparing = { steps: [], at: 0 };

  try {
    const ready = await tried(() =>
      commands.prepareGame(app.gameDir, project, afresh),
    );
    if (ready.status === 'ok') {
      app.survey = ready.data.survey;
      app.faces = ready.data.faces;
      forgetShown();
    }

    return ready.status === 'ok';
  } finally {
    app.preparing = null;
    app.working = false;
  }
}

export async function rereadGame() {
  if (locked()) return;

  await readGame(true);
}

export async function closeProject() {
  if (locked()) return;

  app.working = true;
  try {
    if (!(await saveProject())) return;

    await commands.closeProject();
    shut();
    quiet();
  } finally {
    app.working = false;
  }
}

export async function deleteProject() {
  if (locked()) return;

  await alone(
    () => commands.deleteProject(app.gameDir),
    () => {
      shut();
      quiet();
    },
  );
}

export function adopt(opened: Opened) {
  app.gameDir = opened.gameDir;
  app.project = opened.project;
  app.piles = opened.piles;
  app.sources = opened.sources;
  app.faces = opened.faces;
  app.survey = opened.survey;
  recall(opened.logs);
}

function shut() {
  app.view = 'text';
  app.pane = 'general';
  forgetShown();
  forgetRail();
  app.survey = null;
  app.gameDir = '';
  app.project = null;
  app.piles = false;
  app.sources = [];
  app.faces = [];
  forgetLog();
}

export async function abandon() {
  shut();

  await commands.closeProject();
}
