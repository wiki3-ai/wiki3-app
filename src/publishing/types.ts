/**
 * Publishing workflow types.
 *
 * These mirror the Rust backend types and are used by the frontend
 * to communicate with the Tauri commands.
 */

/** Hosting provider type. */
export type ProviderType = 'github';

/** How the static site is published. */
export type PublishMode = 'docs_folder' | 'gh_pages_branch' | 'none';

/** How the workspace was created. */
export type WorkspaceOrigin =
  | { template: { template_owner: string; template_repo: string } }
  | { fork: { upstream_owner: string; upstream_repo: string } }
  | 'existing';

/** Repository visibility. */
export type RepoVisibility = 'public' | 'private';

/** A named git remote. */
export interface RemoteInfo {
  name: string;
  url: string;
}

/** A workspace represents a local site project connected to a remote. */
export interface Workspace {
  id: string;
  name: string;
  local_path: string;
  provider: ProviderType;
  owner: string;
  repo: string;
  branch: string;
  remotes: RemoteInfo[];
  publish_mode: PublishMode;
  site_url: string | null;
  origin: WorkspaceOrigin;
  visibility: RepoVisibility;
  description: string | null;
  created_at: string;
  last_opened_at: string;
}

/** File change status. */
export type FileStatus = 'modified' | 'added' | 'deleted' | 'renamed' | 'untracked';

/** A file with uncommitted changes. */
export interface DirtyFile {
  path: string;
  status: FileStatus;
}

/** Commit information. */
export interface CommitInfo {
  sha: string;
  message: string;
  author: string;
  date: string;
}

/** Git status summary. */
export interface GitStatus {
  branch: string;
  dirty_files: DirtyFile[];
  ahead: number;
  behind: number;
  last_commit: CommitInfo | null;
}

/** Push operation result. */
export interface PushResult {
  success: boolean;
  remote: string;
  branch: string;
  message: string;
}

/** Publish operation result. */
export interface PublishResult {
  success: boolean;
  site_url: string | null;
  publish_mode: PublishMode;
  message: string;
}

/** GitHub user info. */
export interface GitHubUser {
  login: string;
  name: string | null;
  avatar_url: string | null;
}

/** Authentication status. */
export interface AuthStatus {
  authenticated: boolean;
  user?: GitHubUser;
  reason?: string;
}
