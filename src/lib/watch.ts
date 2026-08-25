export function watch(
  root: Element | null,
  targets: Iterable<Element>,
  seen: (target: Element) => void,
  margin: string,
): () => void {
  const watcher = new IntersectionObserver(
    (entries) => {
      for (const one of entries) {
        if (one.isIntersecting) seen(one.target);
      }
    },
    { root, rootMargin: margin },
  );

  for (const target of targets) watcher.observe(target);

  return () => watcher.disconnect();
}
