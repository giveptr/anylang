import { commands, type Exported } from '$lib/bindings';
import { app, tell } from '$lib/app.svelte';
import { alone, ready, stopScope } from '$lib/project';
import { settle } from '$lib/rail.svelte';
import type { ScopeActions } from '$lib/components/types';

function wrote({ lines, files, reverted }: Exported) {
  if (lines)
    return (
      `${lines} line(s) written across ${files} file(s)` +
      (reverted ? `, ${reverted} put back to the original` : '')
    );

  return reverted
    ? `${reverted} file(s) put back to the original`
    : 'nothing to write yet';
}

export function scopeActions(on: {
  reload: () => Promise<void>;
  cleared: (keys: string[]) => Promise<void>;
}): ScopeActions {
  return {
    ontoggle: (scopes: string[], excluded: boolean) =>
      alone(
        () => commands.excludeScope(app.gameDir, scopes, excluded),
        on.reload,
      ),

    ontranslate: async (scopes: string[]) => {
      if (!ready()) return;

      await alone(
        () => commands.translateScope(app.gameDir, scopes),
        on.reload,
      );
    },

    onexport: async (scopes: string[]) => {
      if (!ready()) return;

      await alone(
        () => commands.exportScope(app.gameDir, scopes),
        (done) => {
          tell(wrote(done));
          settle(done);
        },
      );
    },

    onrevert: (scopes: string[]) =>
      alone(() => commands.revertScope(app.gameDir, scopes), settle),

    onclear: (scopes: string[]) =>
      alone(() => commands.clearScope(app.gameDir, scopes), on.cleared),

    onstop: stopScope,
  };
}
