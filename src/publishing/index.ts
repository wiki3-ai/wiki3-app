/**
 * Wiki3 Publishing Workflow — Public API
 *
 * This module exports the publishing types and API for use by
 * the dashboard controller (main.ts).
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
