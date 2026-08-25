import type { Project } from '$lib/bindings';

export const sourceLabel = (folder: string) =>
  folder ? `game/tl/${folder}` : "The game's original text";

export const shippedOf = (project: Project) =>
  project.tweaks?.kind === 'renpy' ? (project.tweaks.shipped ?? '') : '';

export const setShipped = (project: Project, folder: string) => {
  if (project.tweaks?.kind === 'renpy') project.tweaks.shipped = folder;
};

export const paths = (folders: string[], shipped = '') => [
  ...new Set([...folders, shipped].filter(Boolean)),
  '',
];

export const reads = (project: Project, folders: string[]) =>
  paths(folders, shippedOf(project)).map((one) => ({
    value: one,
    label: sourceLabel(one),
  }));
