const KEY = 'theme';

function wanted(): boolean {
  const kept = localStorage.getItem(KEY);
  if (kept) return kept === 'dark';

  return window.matchMedia('(prefers-color-scheme: dark)').matches;
}

function paint(dark: boolean) {
  document.documentElement.dataset.theme = dark ? 'dark' : 'light';
}

export const theme = $state({ dark: wanted() });

paint(theme.dark);

export function flip() {
  theme.dark = !theme.dark;
  localStorage.setItem(KEY, theme.dark ? 'dark' : 'light');
  paint(theme.dark);
}
