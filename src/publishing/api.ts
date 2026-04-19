/**
 * Publishing API — Tauri command wrappers.
 *
 * Provides a typed interface to all publishing-related Tauri commands.
 */

import { invoke as tauriInvoke } from '@tauri-apps/api/core';

import type {
  AuthStatus,
  CommitInfo,
  GitStatus,
  PublishMode,
  PublishResult,
  PushResult,
  RepoVisibility,
  Workspace,
} from './types';

/** Invoke a Tauri command with typed return. */
async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return tauriInvoke<T>(cmd, args);
}

// =============================================================================
// Auth
// =============================================================================

/** Store a GitHub personal access token and validate it. */
export async function storeGitHubToken(token: string): Promise<AuthStatus> {
  return invoke<AuthStatus>('store_github_token', { token });
}

/** Get current GitHub authentication status. */
export async function getAuthStatus(): Promise<AuthStatus> {
  return invoke<AuthStatus>('get_auth_status');
}

/** Clear stored GitHub credentials. */
export async function clearGitHubAuth(): Promise<{ cleared: boolean }> {
  return invoke<{ cleared: boolean }>('clear_github_auth');
}

// =============================================================================
// Workspaces
// =============================================================================

/** List all known workspaces. */
export async function listWorkspaces(): Promise<Workspace[]> {
  return invoke<Workspace[]>('list_workspaces');
}

/** Get a single workspace by ID. */
export async function getWorkspace(workspaceId: string): Promise<Workspace | null> {
  return invoke<Workspace | null>('get_workspace', { workspaceId });
}

/** Remove a workspace (metadata only). */
export async function removeWorkspace(workspaceId: string): Promise<void> {
  await invoke('remove_workspace', { workspaceId });
}

// =============================================================================
// Create from template
// =============================================================================

export interface CreateFromTemplateOptions {
  owner: string;
  repoName: string;
  visibility: RepoVisibility;
  description?: string;
  templateOwner?: string;
  templateRepo?: string;
}

/** Create a new site from a template repository. */
export async function createSiteFromTemplate(
  options: CreateFromTemplateOptions,
): Promise<Workspace> {
  return invoke<Workspace>('create_site_from_template', {
    owner: options.owner,
    repoName: options.repoName,
    visibility: options.visibility,
    description: options.description ?? null,
    templateOwner: options.templateOwner ?? null,
    templateRepo: options.templateRepo ?? null,
  });
}

// =============================================================================
// Fork
// =============================================================================

export interface ForkSiteOptions {
  sourceOwner: string;
  sourceRepo: string;
  targetOwner?: string;
  forkName?: string;
}

/** Fork an existing repository and clone it locally. */
export async function forkSite(options: ForkSiteOptions): Promise<Workspace> {
  return invoke<Workspace>('fork_site', {
    sourceOwner: options.sourceOwner,
    sourceRepo: options.sourceRepo,
    targetOwner: options.targetOwner ?? null,
    forkName: options.forkName ?? null,
  });
}

// =============================================================================
// Git operations
// =============================================================================

/** Get git status for a workspace. */
export async function getGitStatus(workspaceId: string): Promise<GitStatus> {
  return invoke<GitStatus>('get_git_status', { workspaceId });
}

/** Commit all changes in a workspace. */
export async function commitChanges(
  workspaceId: string,
  message: string,
): Promise<CommitInfo> {
  return invoke<CommitInfo>('commit_changes', { workspaceId, message });
}

/** Push current branch to origin. */
export async function pushChanges(workspaceId: string): Promise<PushResult> {
  return invoke<PushResult>('push_changes', { workspaceId });
}

/** Commit and push in one operation. */
export async function commitAndPush(
  workspaceId: string,
  message: string,
): Promise<{ commit: CommitInfo; push: PushResult }> {
  return invoke<{ commit: CommitInfo; push: PushResult }>('commit_and_push', {
    workspaceId,
    message,
  });
}

// =============================================================================
// Publish
// =============================================================================

/** Publish/update the site. */
export async function publishSite(workspaceId: string): Promise<PublishResult> {
  return invoke<PublishResult>('publish_site', { workspaceId });
}

/** Detect the publish mode for a workspace. */
export async function detectPublishMode(workspaceId: string): Promise<PublishMode> {
  return invoke<PublishMode>('detect_workspace_publish_mode', { workspaceId });
}

// =============================================================================
// Open existing workspace
// =============================================================================

/** Register an existing local git repo as a workspace. */
export async function openLocalWorkspace(localPath: string): Promise<Workspace> {
  return invoke<Workspace>('open_local_workspace', { localPath });
}

// =============================================================================
// Open site from repo URL
// =============================================================================

/** Resolve a GitHub repo URL to its Pages site and open it in a new window. */
export async function openRepoSite(repoUrl: string): Promise<{ owner: string; repo: string; site_url: string }> {
  return invoke<{ owner: string; repo: string; site_url: string }>('open_repo_site', { repoUrl });
}

// =============================================================================
// Settings
// =============================================================================

export interface AppSettings {
  restore_windows: boolean;
  default_repo_url: string;
}

/** Get the current app settings. */
export async function getSettings(): Promise<AppSettings> {
  return invoke<AppSettings>('get_settings');
}

/** Update app settings (partial). Returns the full updated settings. */
export async function updateSettings(
  restoreWindows?: boolean,
  defaultRepoUrl?: string,
): Promise<AppSettings> {
  return invoke<AppSettings>('update_settings', {
    restoreWindows: restoreWindows ?? null,
    defaultRepoUrl: defaultRepoUrl ?? null,
  });
}
