import type { Swap } from '$lib/bindings';

export type { Swap };

export const swapFor = (swaps: Swap[], from: string) =>
  swaps.find((one) => one.from === from)?.to ?? '';

export const withSwap = (swaps: Swap[], from: string, to: string): Swap[] => {
  const rest = swaps.filter((one) => one.from !== from);

  return to ? [...rest, { from, to }] : rest;
};

export const filled = (swaps: Swap[]) =>
  swaps.filter((one) => one.to.trim()).length;
