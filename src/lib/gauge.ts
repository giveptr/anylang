export function gauged(
  box: HTMLDivElement | null,
  inside: HTMLElement | null,
  say: (long: number, view: number) => void,
) {
  if (!box) return;

  const tell = () => say(box.scrollHeight, box.clientHeight);
  const watch = new ResizeObserver(tell);

  watch.observe(box);
  if (inside) watch.observe(inside);
  tell();

  return () => watch.disconnect();
}
