export type Kept = 'every' | 'replaced' | 'marked';

export type Narrow = {
  query: string;
  spot: string;
  atlas: string;
  kept: Kept;
};

export type Zoomed = 'original' | 'replacement';

export const lineName = (one: { file: string; id: number }) =>
  `${one.file}\u0000${one.id}`;

export const lineId = (one: { file: string; id: number }) =>
  `line-${one.file}-${one.id}`;

export type Target = {
  at: string;
  scopes: string[];
  excluded: boolean;
  applied: boolean;
  translated: number;
  busy: boolean;
};

export type ScopeActions = {
  ontoggle: (scopes: string[], excluded: boolean) => void;
  ontranslate: (scopes: string[]) => void;
  onexport: (scopes: string[]) => void;
  onrevert: (scopes: string[]) => void;
  onclear: (scopes: string[]) => void;
  onstop: (scopes: string[]) => void;
};
