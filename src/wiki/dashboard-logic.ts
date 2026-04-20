/**
 * Pure helpers that drive dashboard interactions.
 *
 * These are factored out of `main.ts` so they can be unit-tested
 * without a DOM or Tauri runtime. Two real bugs that prior releases
 * shipped were silent-noop paths in drag-reorder and clone-target
 * resolution; the corresponding logic lives here behind tests.
 */

import type { Wiki } from './types';

/**
 * Compute the new wiki ordering after dragging `srcId` onto `dstId`.
 *
 * @param before - if true, insert the source above the target; if false,
 *   insert below. The caller derives this from the cursor's position
 *   within the drop target's bounding rect.
 *
 * Returns the input list unchanged when the move is a no-op (same id,
 * unknown id, or the computed position equals the current one).
 */
export function computeReorder(
  wikis: readonly Wiki[],
  srcId: string,
  dstId: string,
  before: boolean,
): Wiki[] {
  if (srcId === dstId) return wikis.slice();
  const srcIdx = wikis.findIndex((w) => w.id === srcId);
  const dstIdx = wikis.findIndex((w) => w.id === dstId);
  if (srcIdx < 0 || dstIdx < 0) return wikis.slice();

  const next = wikis.slice();
  const [moved] = next.splice(srcIdx, 1);

  // Recompute destination index *after* removing the source.
  let insertAt = next.findIndex((w) => w.id === dstId);
  if (insertAt < 0) insertAt = next.length;
  if (!before) insertAt += 1;

  next.splice(insertAt, 0, moved);
  return next;
}

/**
 * Resolution strategy for the clone target picker. The frontend calls
 * this after the user has picked a directory from the native file
 * dialog; the result decides whether to clone directly into the picked
 * directory or ask for a sub-folder name.
 */
export type CloneTargetResolution =
  | { kind: 'cancelled' }
  | { kind: 'use_directly'; path: string }
  | { kind: 'needs_subfolder'; parent: string; defaultName: string };

/**
 * Decide how to use a directory the user just picked as a clone target.
 *
 * - If they cancelled the picker, return `cancelled`.
 * - If they picked an empty directory, clone directly into it —
 *   this matches the natural "create a folder in the dialog, select
 *   it, done" mental model.
 * - If they picked a non-empty directory, we need a subfolder name;
 *   the caller should prompt for one.
 */
export function resolveCloneTarget(
  picked: string | null,
  isEmpty: boolean,
  defaultName: string,
): CloneTargetResolution {
  if (!picked) return { kind: 'cancelled' };
  if (isEmpty) return { kind: 'use_directly', path: picked };
  return { kind: 'needs_subfolder', parent: picked, defaultName };
}

/**
 * Join a parent directory and a sub-folder name into a full path,
 * using the platform separator inferred from the parent.
 */
export function joinPath(parent: string, child: string): string {
  const trimmed = child.trim();
  if (!trimmed) return parent;
  const sep = parent.includes('\\') && !parent.includes('/') ? '\\' : '/';
  if (parent.endsWith('/') || parent.endsWith('\\')) {
    return `${parent}${trimmed}`;
  }
  return `${parent}${sep}${trimmed}`;
}

/**
 * Derive a default sub-folder name from a remote URL, falling back to
 * `my-garden` when nothing can be extracted.
 */
export function defaultNameFromRemote(url: string): string {
  const m = url.trim().match(/[:/]([^/:]+?)(?:\.git)?\/?$/);
  return m ? m[1] : 'my-garden';
}
