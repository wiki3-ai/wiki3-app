/**
 * Type definitions for the Wiki3 desktop integration layer.
 */

/** Permission choices for desktop execution gating. */
export type PermissionChoice = 'allow_once' | 'allow_always' | 'deny';

/** Host detection result. */
export interface HostInfo {
  detected: boolean;
  host?: string;
  version?: string;
  origin?: string;
  reason?: string;
}

/** Permission state for an origin. */
export interface PermissionState {
  origin: string;
  execution_allowed: boolean;
  choice: PermissionChoice | null;
}

/** Execution enablement state from the execution policy layer. */
export interface ExecutionState {
  trusted: boolean;
  execution_allowed: boolean;
  needs_permission?: boolean;
  reason?: string;
}

/** App configuration (non-sensitive). */
export interface AppConfig {
  site_url: string;
  trusted_origins: string[];
  version: string;
}

/** Desktop integration state tracked by the extension. */
export interface DesktopIntegrationState {
  /** Whether we're running in the desktop app. */
  isDesktop: boolean;
  /** Host info from detection. */
  hostInfo: HostInfo | null;
  /** Current permission state. */
  permissionState: PermissionState | null;
  /** Current execution state. */
  executionState: ExecutionState | null;
  /** Whether initialization is complete. */
  initialized: boolean;
}
