import { getCurrentWindow } from '@tauri-apps/api/window';

const win = getCurrentWindow();

export const frame = $state({ maximized: false });

async function asked(question: () => Promise<boolean>) {
  return question().catch(() => false);
}

async function look() {
  const [big, whole] = await Promise.all([
    asked(() => win.isMaximized()),
    asked(() => win.isFullscreen()),
  ]);

  frame.maximized = big || whole;
}

look();
window.addEventListener('resize', look);
window.addEventListener('focus', look);

export function reveal() {
  void win.show().then(() => win.setFocus());
}

export const minimize = () => win.minimize();
export const toggle = () => win.toggleMaximize();
export const close = () => win.close();
