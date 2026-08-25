import type { Only } from '$lib/bindings';

export const TRANSLATE = 'AI translate';
export const APPLY = 'Apply to game';
export const NOTHING_TO_APPLY = 'Nothing to apply';
export const RESTORE = 'Restore original files';

export const READ_FROM = 'Read the text from';

export const KINDS: readonly (readonly [Only, string])[] = [
  ['yours', 'All lines'],
  ['asked', 'Words'],
  ['listed', 'Maybe not words'],
];

export const namedKind = (wanted: Only) =>
  KINDS.find(([option]) => option === wanted)?.[1] ?? '';

export const LISTED_HINT = `${namedKind('listed')}. Translating it may break the game.`;

export const NO_PIXELS = 'This reader cannot reach the pixels behind this one';

export const NO_ASKED = 'No words to translate here';
export const NO_LISTED = 'No doubtful lines here';

export const CLEAR_ARMED = 'Yes, clear them';
export const CLEAR_WARNING =
  'Deletes the lines translated here and puts these game files back to the originals.';
