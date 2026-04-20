/**
 * Typed wrappers around the wiki / window Tauri commands.
 */

import { invoke as tauriInvoke } from '@tauri-apps/api/core';
import { open as openDialog } from '@tauri-apps/plugin-dialog';

import type {
  AddWikiParams,
  TrackedWindowInfo,
  UpdateWikiParams,
  Wiki,
} from './types';

function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return tauriInvoke<T>(cmd, args);
}

// ── Wiki CRUD ────────────────────────────────────────────────────────────

export function listWikis(): Promise<Wiki[]> {
  return invoke<Wiki[]>('list_wikis');
}

export function getWiki(wikiId: string): Promise<Wiki | null> {
  return invoke<Wiki | null>('get_wiki', { wikiId });
}

export function addWiki(params: AddWikiParams): Promise<Wiki> {
  return invoke<Wiki>('add_wiki', { params });
}

export function updateWiki(wikiId: string, params: UpdateWikiParams): Promise<Wiki> {
  return invoke<Wiki>('update_wiki', { wikiId, params });
}

export function removeWiki(wikiId: string): Promise<void> {
  return invoke<void>('remove_wiki', { wikiId });
}

export function restoreDefaultWikis(): Promise<Wiki[]> {
  return invoke<Wiki[]>('restore_default_wikis');
}

export function getDefaultWikisDir(): Promise<string> {
  return invoke<string>('get_default_wikis_dir');
}

// ── Wiki actions ────────────────────────────────────────────────────────

/** Open the wiki's site URL in a new in-app window (tagged to the wiki). */
export function openWikiSite(wikiId: string): Promise<string> {
  return invoke<string>('open_wiki_site', { wikiId });
}

/** Open the wiki's remote repo URL in the system browser. */
export function openWikiRemote(wikiId: string): Promise<string> {
  return invoke<string>('open_wiki_remote', { wikiId });
}

/** Reveal the wiki's local path in the OS file manager. */
export function revealWikiLocal(wikiId: string): Promise<string> {
  return invoke<string>('reveal_wiki_local', { wikiId });
}

/** Register an existing local git repo as a new wiki. */
export function openLocalRepoAsWiki(localPath: string): Promise<Wiki> {
  return invoke<Wiki>('open_local_repo_as_wiki', { localPath });
}

/** Clone a remote repo to a chosen local folder and register it as a wiki. */
export function cloneWiki(remoteUrl: string, targetPath: string): Promise<Wiki> {
  return invoke<Wiki>('clone_wiki', { remoteUrl, targetPath });
}

// ── Per-wiki window tracking ─────────────────────────────────────────────

export function listWikiWindows(wikiId: string): Promise<TrackedWindowInfo[]> {
  return invoke<TrackedWindowInfo[]>('list_wiki_windows', { wikiId });
}

export function listAllTrackedWindows(): Promise<TrackedWindowInfo[]> {
  return invoke<TrackedWindowInfo[]>('list_all_tracked_windows');
}

export function closeWikiWindows(wikiId: string): Promise<number> {
  return invoke<number>('close_wiki_windows', { wikiId });
}

export function reopenWikiWindows(wikiId: string): Promise<number> {
  return invoke<number>('reopen_wiki_windows', { wikiId });
}

export function focusWindow(label: string): Promise<void> {
  return invoke<void>('focus_window', { label });
}

export function forgetTrackedWindow(label: string): Promise<void> {
  return invoke<void>('forget_tracked_window', { label });
}

// ── Dashboard lifecycle ──────────────────────────────────────────────────

export function toggleDashboard(): Promise<void> {
  return invoke<void>('toggle_dashboard');
}

// ── External ─────────────────────────────────────────────────────────────

export function openExternalUrl(url: string): Promise<void> {
  return invoke<void>('open_external_url', { url });
}

// ── File dialog helpers ──────────────────────────────────────────────────

/** Let the user pick an existing local folder. Returns `null` if cancelled. */
export async function pickFolder(defaultPath?: string): Promise<string | null> {
  const result = await openDialog({
    directory: true,
    multiple: false,
    defaultPath,
  });
  if (typeof result === 'string') return result;
  return null;
}

/**
 * Let the user choose a *target* folder for a clone by selecting a parent
 * directory plus a folder name. We use an existing-folder picker (which
 * returns the parent) and then prompt for the sub-folder name; the result
 * is the full absolute target path. Returns `null` if cancelled.
 */
export async function pickCloneTarget(
  defaultBase: string,
  defaultName: string,
): Promise<string | null> {
  const parent = await pickFolder(defaultBase);
  if (!parent) return null;
  const folderName = window.prompt('Folder name for the wiki:', defaultName);
  if (!folderName) return null;
  const trimmed = folderName.trim();
  if (!trimmed) return null;
  // Use forward slash; Rust's Path handles both on Windows too, but prefer
  // the platform separator where possible.
  const sep = parent.includes('\\') ? '\\' : '/';
  const joined =
    parent.endsWith('/') || parent.endsWith('\\')
      ? `${parent}${trimmed}`
      : `${parent}${sep}${trimmed}`;
  return joined;
}
