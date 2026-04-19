/**
 * Wiki3 Publishing Workflow — Public API
 *
 * This module exports the publishing components for creating, managing,
 * and publishing wiki3 sites from the desktop app.
 */

// API layer
export * as publishingApi from './api';

// Types
export type {
  AuthStatus,
  CommitInfo,
  DirtyFile,
  FileStatus,
  GitHubUser,
  GitStatus,
  ProviderType,
  PublishMode,
  PublishResult,
  PushResult,
  RemoteInfo,
  RepoVisibility,
  Workspace,
  WorkspaceOrigin,
} from './types';

// UI components
export { AuthPanel } from './ui/auth-panel';
export { CommitPushPanel } from './ui/commit-push-panel';
export { ForkSiteDialog } from './ui/fork-dialog';
export { NewSiteDialog } from './ui/new-site-dialog';
export { PublishPanel } from './ui/publish-panel';
export { WorkspacePanel } from './ui/workspace-panel';
