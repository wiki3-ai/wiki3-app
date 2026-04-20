/**
 * Wiki dashboard entry types (mirrors `src-tauri/src/wiki/types.rs`).
 */

export type WikiProvider = 'git_hub' | 'other';

export type WikiVisibility = 'public' | 'private' | 'unknown';

export type WikiOrigin =
  | 'seeded'
  | 'manual'
  | 'existing'
  | 'clone'
  | { template: { template_owner: string; template_repo: string } }
  | { fork: { upstream_owner: string; upstream_repo: string } };

export interface RemoteRef {
  provider: WikiProvider;
  owner: string;
  repo: string;
  url: string;
  visibility: WikiVisibility;
}

export interface Wiki {
  id: string;
  name: string;
  local_path: string | null;
  remote: RemoteRef | null;
  site_url: string | null;
  origin: WikiOrigin;
  description: string | null;
  created_at: string;
  last_opened_at: string;
  publish_on_commit: boolean;
}

export interface AddWikiParams {
  name?: string | null;
  local_path?: string | null;
  remote_url?: string | null;
  site_url?: string | null;
  description?: string | null;
}

export interface UpdateWikiParams {
  name?: string | null;
  /** `null` clears, `undefined` leaves unchanged. */
  local_path?: string | null;
  remote_url?: string | null;
  site_url?: string | null;
  description?: string | null;
  publish_on_commit?: boolean;
}

export interface GitStatus {
  branch: string;
  ahead: number;
  behind: number;
  staged_files: string[];
  dirty_files: string[];
  untracked_files: string[];
}

export interface CommitInfo {
  sha: string;
  message: string;
  author: string;
  timestamp: string;
}

export interface PushResult {
  success: boolean;
  branch: string;
  message: string;
}

export interface TrackedWindowInfo {
  label: string;
  url: string;
  wiki_id: string | null;
  closed: boolean;
  x: number;
  y: number;
  width: number;
  height: number;
}
