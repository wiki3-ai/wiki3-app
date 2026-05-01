/**
 * Typed wrappers around the wiki / window Tauri commands.
 */

import { invoke as tauriInvoke } from '@tauri-apps/api/core';
import { open as openDialog } from '@tauri-apps/plugin-dialog';

import type {
  AddWikiParams,
  CommitInfo,
  GitStatus,
  PushResult,
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

/** Persist a new order for the dashboard. Any omitted ids are kept at the end. */
export function reorderWikis(order: string[]): Promise<void> {
  return invoke<void>('reorder_wikis', { order });
}

/** Toggle the "Publish on Commit" flag for a wiki. */
export function setWikiPublishOnCommit(wikiId: string, value: boolean): Promise<Wiki> {
  return invoke<Wiki>('set_wiki_publish_on_commit', { wikiId, value });
}

/** Toggle the "Autostart preview container" flag for a wiki. */
export function setWikiAutostartContainer(wikiId: string, value: boolean): Promise<Wiki> {
  return invoke<Wiki>('set_wiki_autostart_container', { wikiId, value });
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

// ── Per-wiki git + publish ───────────────────────────────────────────────

export function wikiGitStatus(wikiId: string): Promise<GitStatus> {
  return invoke<GitStatus>('wiki_git_status', { wikiId });
}

export function wikiCommit(wikiId: string, message: string): Promise<{ commit: CommitInfo }> {
  return invoke<{ commit: CommitInfo }>('wiki_commit', { wikiId, message });
}

export function wikiPush(wikiId: string): Promise<PushResult> {
  return invoke<PushResult>('wiki_push', { wikiId });
}

export function wikiPull(wikiId: string): Promise<string> {
  return invoke<string>('wiki_pull', { wikiId });
}

export function wikiPublish(
  wikiId: string,
): Promise<{ push: PushResult; site_url: string | null }> {
  return invoke<{ push: PushResult; site_url: string | null }>('wiki_publish', { wikiId });
}

/** Commit; if `alsoPublish` (or the wiki's stored `publish_on_commit`) is true, also push + publish. */
export function wikiCommitAndMaybePublish(
  wikiId: string,
  message: string,
  alsoPublish?: boolean,
): Promise<{ committed: boolean; published: boolean; commit: unknown; publish?: unknown }> {
  return invoke<{
    committed: boolean;
    published: boolean;
    commit: unknown;
    publish?: unknown;
  }>('wiki_commit_and_maybe_publish', {
    wikiId,
    message,
    alsoPublish: alsoPublish ?? null,
  });
}

/** Run `jupyter lite build` in the wiki's local directory. */
export function wikiBuildSite(
  wikiId: string,
): Promise<{ success: boolean; output_dir: string; stdout: string; stderr: string }> {
  return invoke<{ success: boolean; output_dir: string; stdout: string; stderr: string }>(
    'wiki_build_site',
    { wikiId },
  );
}

/**
 * Start (or re-attach to) the per-wiki preview container. Idempotent:
 * returns the existing running site if already started. Returns the
 * loopback URL once the serve port is accepting connections.
 */
export function wikiStartContainer(wikiId: string): Promise<RunningSite> {
  return invoke<RunningSite>('wiki_start_container', { wikiId });
}

/** Stop the preview container for a wiki (best-effort). */
export function wikiStopContainer(wikiId: string): Promise<void> {
  return invoke<void>('wiki_stop_container', { wikiId });
}

export interface RunningSite {
  wiki_id: string;
  serve_container: string;
  watch_container: string | null;
  host_port: number;
  url: string;
}

export function wikiContainerStatus(wikiId: string): Promise<RunningSite | null> {
  return invoke<RunningSite | null>('wiki_container_status', { wikiId });
}

// ── Generic container controls (devcontainer-core) ───────────────────
//
// Thin wrappers around the new `wiki_container_ctl_*` Tauri commands.
// These are independent of the wiki-specific `Serve` flow above and
// drive the start/stop/restart/rebuild/remove buttons on each card.

export interface ContainerControlStatus {
  wikiId: string;
  state: string;
  containerId?: string | null;
  imageRef?: string | null;
  error?: string | null;
  configDrift?: boolean;
}

export function wikiContainerCtlStatus(wikiId: string): Promise<ContainerControlStatus> {
  return invoke<ContainerControlStatus>('wiki_container_ctl_status', { wikiId });
}
export function wikiContainerCtlUp(wikiId: string): Promise<ContainerControlStatus> {
  return invoke<ContainerControlStatus>('wiki_container_ctl_up', { wikiId });
}
export function wikiContainerCtlStop(wikiId: string): Promise<ContainerControlStatus> {
  return invoke<ContainerControlStatus>('wiki_container_ctl_stop', { wikiId });
}
export function wikiContainerCtlRestart(wikiId: string): Promise<ContainerControlStatus> {
  return invoke<ContainerControlStatus>('wiki_container_ctl_restart', { wikiId });
}
export function wikiContainerCtlRebuild(wikiId: string): Promise<ContainerControlStatus> {
  return invoke<ContainerControlStatus>('wiki_container_ctl_rebuild', { wikiId });
}
export function wikiContainerCtlRemove(wikiId: string): Promise<ContainerControlStatus> {
  return invoke<ContainerControlStatus>('wiki_container_ctl_remove', { wikiId });
}
/** Cancel any in-flight lifecycle hook (e.g. a stuck postCreateCommand). */
export function wikiContainerCtlCancel(wikiId: string): Promise<boolean> {
  return invoke<boolean>('wiki_container_ctl_cancel', { wikiId });
}

/** Open an arbitrary URL in a new in-app window tagged to a wiki. */
export function openNewWindowForWiki(url: string, wikiId: string): Promise<void> {
  return invoke<void>('open_new_window_for_wiki', { url, wikiId });
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

/** Return whether a directory path exists and has no entries. */
export function isEmptyDir(path: string): Promise<boolean> {
  return invoke<boolean>('is_empty_dir', { path });
}

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
 * Let the user choose a *target* folder for a clone.
 *
 * If the user picks an empty directory we use it directly — that's the
 * natural "create and select this folder in the dialog" flow. Only when
 * they pick a *non-empty* directory do we prompt for a subfolder name;
 * the result is the full absolute target path.
 *
 * Returns `null` if the user cancelled at any step.
 */
export async function pickCloneTarget(
  defaultBase: string,
  defaultName: string,
): Promise<string | null> {
  const picked = await pickFolder(defaultBase);
  if (!picked) return null;

  // If it's empty (including "just created in the dialog"), use as-is.
  const empty = await isEmptyDir(picked).catch(() => false);
  if (empty) return picked;

  const folderName = window.prompt(
    `"${picked}" is not empty. Enter a sub-folder name to clone into:`,
    defaultName,
  );
  if (!folderName || !folderName.trim()) return null;
  return joinPath(picked, folderName.trim());
}

function joinPath(parent: string, child: string): string {
  const sep = parent.includes('\\') && !parent.includes('/') ? '\\' : '/';
  if (parent.endsWith('/') || parent.endsWith('\\')) return `${parent}${child}`;
  return `${parent}${sep}${child}`;
}
