import { getName, getVersion } from '@tauri-apps/api/app';
import { commands } from '$lib/bindings';

export const GITHUB = 'https://github.com/giveptr/anylang';

const FEW = 4;

export const about = $state({
  name: '',
  version: '',
  kinds: [] as string[],
  atOnce: FEW,
});

export async function learn() {
  if (about.name) return;

  const [name, version, kinds, atOnce] = await Promise.all([
    getName(),
    getVersion(),
    commands.pictureKinds(),
    commands.picturesAtOnce(),
  ]);

  about.name = name;
  about.version = version;
  about.kinds = kinds;
  about.atOnce = atOnce;
}
