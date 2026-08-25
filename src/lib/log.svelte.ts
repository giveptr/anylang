import { commands, events, type LogEntry, type Notice } from '$lib/bindings';
import { alarm } from '$lib/app.svelte';
import { caught } from '$lib/save';

const KEPT = 20_000;

export const log = $state({
  lines: [] as LogEntry[],
  warnings: 0,
  errors: 0,
  unseen: false,
});

function counted(entry: LogEntry, by: 1 | -1) {
  if (entry.level === 'warn') log.warnings += by;
  else if (entry.level === 'error') log.errors += by;
}

function keep(entry: Notice) {
  log.lines.push(entry);
  counted(entry, 1);
  if (entry.level === 'error') log.unseen = true;

  while (log.lines.length > KEPT) {
    const gone = log.lines.shift();
    if (gone) counted(gone, -1);
  }
}

export function listen() {
  return events.notice.listen(({ payload }) => keep(payload));
}

export function recall(kept: LogEntry[]) {
  log.lines = kept;
  log.warnings = kept.filter((one) => one.level === 'warn').length;
  log.errors = kept.filter((one) => one.level === 'error').length;
  log.unseen = false;
}

export function forgetLog() {
  log.lines = [];
  log.warnings = 0;
  log.errors = 0;
  log.unseen = false;
}

export function saw() {
  log.unseen = false;
}

export async function wipe(gameDir: string) {
  forgetLog();
  if (!gameDir) return;

  const done = await caught(() => commands.forgetLogs(gameDir));
  if (done.status === 'error') alarm(done.error);
}
