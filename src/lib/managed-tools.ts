/**
 * Managed-tools client for the Wiki3 desktop app.
 *
 * Wraps the Rust-side `tools_*` and `detect_apple_container` Tauri
 * commands so the dashboard / first-run UI can drive installs,
 * status checks, and uninstalls without knowing the wire format.
 *
 * Events:
 *   `wiki3://tools/install-progress` — streamed during `ensure`.
 *   `wiki3://tools/install-done`     — emitted on successful install.
 *
 * These are emitted by the Rust side via `tauri::Emitter`; subscribe
 * with the standard `@tauri-apps/api/event` listener.
 */
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export const TOOL_NAMES = ['deno'] as const;
export type ToolName = (typeof TOOL_NAMES)[number];

export type ToolStatus =
  | { kind: 'not_installed' }
  | { kind: 'up_to_date'; version: string }
  | { kind: 'out_of_date'; installed: string; pinned: string };

export interface ToolStatusEntry {
  name: string;
  pinned_version: string;
  status: ToolStatus;
}

export interface AppleContainerStatus {
  installed: boolean;
  path: string | null;
}

export type InstallProgress =
  | { phase: 'starting'; name: string; version: string }
  | { phase: 'cache_hit'; name: string; version: string }
  | {
      phase: 'downloading';
      name: string;
      downloaded: number;
      total: number | null;
    }
  | { phase: 'verifying'; name: string }
  | { phase: 'extracting'; name: string }
  | { phase: 'done'; name: string; version: string };

export interface InstallDonePayload {
  name: string;
  version: string;
  path: string;
}

/**
 * Return every registered tool's pinned version + current on-disk status.
 */
export function toolsStatus(): Promise<ToolStatusEntry[]> {
  return invoke<ToolStatusEntry[]>('tools_status');
}

/**
 * Ensure the named tool is installed. Resolves with the absolute path
 * of the managed executable. Subsequent calls are an immediate cache
 * hit — safe and cheap to call from multiple UI entry points.
 */
export function toolsEnsure(name: ToolName): Promise<string> {
  return invoke<string>('tools_ensure', { name });
}

/**
 * Remove a single tool's install tree. Idempotent.
 */
export function toolsUninstall(name: ToolName): Promise<void> {
  return invoke<void>('tools_uninstall', { name });
}

/**
 * Remove every managed tool. Idempotent. Does not touch Apple
 * Container — that's an OS-level installation.
 */
export function toolsUninstallAll(): Promise<void> {
  return invoke<void>('tools_uninstall_all');
}

/**
 * Resolve the path of an installed tool without doing any work.
 * Returns null if not installed.
 */
export function toolsResolve(name: ToolName): Promise<string | null> {
  return invoke<string | null>('tools_resolve', { name });
}

/**
 * Probe for Apple Container. Checks `/usr/local/bin/container`,
 * `/opt/homebrew/bin/container`, then `$PATH`. The resolved path is
 * remembered on the Rust side so runner invocations can reuse it.
 */
export function detectAppleContainer(): Promise<AppleContainerStatus> {
  return invoke<AppleContainerStatus>('detect_apple_container');
}

/**
 * Subscribe to install-progress events for the active ensure()
 * call. Returns an unlisten function.
 */
export function onInstallProgress(
  handler: (p: InstallProgress) => void,
): Promise<UnlistenFn> {
  return listen<InstallProgress>('wiki3://tools/install-progress', (e) =>
    handler(e.payload),
  );
}

/**
 * Subscribe to the terminal "install finished" event.
 */
export function onInstallDone(
  handler: (p: InstallDonePayload) => void,
): Promise<UnlistenFn> {
  return listen<InstallDonePayload>('wiki3://tools/install-done', (e) =>
    handler(e.payload),
  );
}
