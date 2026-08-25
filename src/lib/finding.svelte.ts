const KEY = 'finding';

export const finding = $state({ open: localStorage.getItem(KEY) !== 'closed' });

export function remember(open: boolean) {
  localStorage.setItem(KEY, open ? 'open' : 'closed');
}
