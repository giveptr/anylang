import { commands } from '$lib/bindings';
import { alarm, app, quiet } from '$lib/app.svelte';

export type Outcome<T> =
  { status: 'ok'; data: T } | { status: 'error'; error: string };

const blamed = (blew: unknown) =>
  blew instanceof Error ? blew.message : String(blew);

export async function caught<T>(
  work: () => Promise<Outcome<T>>,
): Promise<Outcome<T>> {
  try {
    return await work();
  } catch (blew) {
    return { status: 'error', error: blamed(blew) };
  }
}

export async function tried<T>(work: () => Promise<Outcome<T>>) {
  quiet();

  const result = await caught(work);
  if (result.status === 'error') alarm(result.error);

  return result;
}

export async function saveProject() {
  const project = app.project;
  if (!project || !app.gameDir) return true;

  const saved = await tried(() => commands.saveProject(app.gameDir, project));

  return saved.status === 'ok';
}
