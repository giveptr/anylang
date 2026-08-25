export type View = 'text' | 'pictures' | 'settings';

export const views: { view: View; label: string }[] = [
  { view: 'text', label: 'Text' },
  { view: 'pictures', label: 'Pictures' },
  { view: 'settings', label: 'Settings' },
];
