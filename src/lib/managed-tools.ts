/**
 * Bundled-tools client for the Wiki3 desktop app.
 *
 * The Deno binary ships **inside the installed .app** (placed there at
 * build time by `build.rs`). There is no user-facing install step, so
 * this client deliberately has no `install`/`uninstall`/`update`
 * methods — only read-only status and cache management.
 */
import { invoke } from '@tauri-apps/api/core';

/** One entry per tool shipped with the app. */
export interface ToolStatusEntry {
  name: string;
  version: string;
  /** Always true; kept as an explicit flag for the UI. */
  bundled: boolean;
  /** Absolute path of the bundled binary, or null if the resource is
   *  missing (which means this app build is broken). */
  path: string | null;
}

export interface AppleContainerStatus {
  installed: boolean;
  path: string | null;
}

export interface CacheInfo {
  path: string;
  exists: boolean;
  size_bytes: number;
}

/**
 * Report bundled-tool status. Currently returns one entry for Deno.
 */
export function toolsStatus(): Promise<ToolStatusEntry[]> {
  return invoke<ToolStatusEntry[]>('tools_status');
}

/**
 * Resolve the path of the bundled Deno binary, or null if it is
 * missing from this app build.
 */
export function toolsBundledDenoPath(): Promise<string | null> {
  return invoke<string | null>('tools_bundled_deno_path');
}

/**
 * Probe for Apple Container. Checks the standard install locations
 * and then `$PATH`. Result is memoized on the Rust side so later
 * build invocations can reuse it.
 */
export function detectAppleContainer(): Promise<AppleContainerStatus> {
  return invoke<AppleContainerStatus>('detect_apple_container');
}

/**
 * Report the location and size of the disposable tools cache (Deno
 * module cache + npm packages for `@devcontainers/cli`). Safe to
 * clear at any time.
 */
export function toolsCacheInfo(): Promise<CacheInfo> {
  return invoke<CacheInfo>('tools_cache_info');
}

/**
 * Delete the tools cache. Idempotent. Never touches the bundled Deno
 * binary (which lives in the read-only Resources directory).
 */
export function toolsClearCache(): Promise<void> {
  return invoke<void>('tools_clear_cache');
}
