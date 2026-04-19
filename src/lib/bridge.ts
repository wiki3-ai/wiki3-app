/**
 * Wiki3 Desktop Host Bridge
 *
 * Provides the communication layer between the JupyterLab/JupyterLite frontend
 * running in the wiki3.ai site and the Tauri desktop host.
 *
 * This module detects desktop host presence, manages permissions,
 * and mediates execution policy — all restricted to trusted origins.
 */

import type { PermissionChoice, PermissionState, HostInfo, ExecutionState } from './types';

/**
 * Check if the Tauri invoke API is available (i.e., we're running inside the desktop app).
 */
function getTauriInvoke(): ((cmd: string, args?: Record<string, unknown>) => Promise<unknown>) | null {
  try {
    // Tauri 2 injects __TAURI_INTERNALS__ on the window object
    const internals = (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ as
      | { invoke?: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> }
      | undefined;
    if (internals?.invoke) {
      return internals.invoke.bind(internals);
    }
  } catch {
    // Not in a Tauri context
  }
  return null;
}

/**
 * Desktop Host Bridge — the main interface for desktop integration.
 *
 * Usage:
 *   const bridge = new DesktopHostBridge();
 *   const info = await bridge.detectHost();
 *   if (info.detected) { ... }
 */
export class DesktopHostBridge {
  private invoke: ((cmd: string, args?: Record<string, unknown>) => Promise<unknown>) | null;
  private origin: string;

  constructor() {
    this.invoke = getTauriInvoke();
    this.origin = window.location.origin;
  }

  /**
   * Whether we appear to be running inside the Tauri desktop app.
   */
  get isDesktop(): boolean {
    return this.invoke !== null;
  }

  /**
   * Detect whether the desktop host is available and this origin is trusted.
   */
  async detectHost(): Promise<HostInfo> {
    if (!this.invoke) {
      return {
        detected: false,
        reason: 'not_in_desktop_app',
      };
    }

    try {
      const result = await this.invoke('detect_desktop_host', {
        origin: this.origin,
      });
      return result as HostInfo;
    } catch (err) {
      console.error('[wiki3-desktop] Host detection failed:', err);
      return {
        detected: false,
        reason: 'detection_error',
      };
    }
  }

  /**
   * Get the current permission state for this origin.
   */
  async getPermissionState(): Promise<PermissionState | null> {
    if (!this.invoke) return null;

    try {
      const result = await this.invoke('get_permission_state', {
        origin: this.origin,
      });
      return result as PermissionState;
    } catch (err) {
      console.error('[wiki3-desktop] Failed to get permission state:', err);
      return null;
    }
  }

  /**
   * Set the execution permission for this origin.
   */
  async setPermission(choice: PermissionChoice): Promise<PermissionState | null> {
    if (!this.invoke) return null;

    try {
      const result = await this.invoke('set_execution_permission', {
        origin: this.origin,
        choice,
      });
      return result as PermissionState;
    } catch (err) {
      console.error('[wiki3-desktop] Failed to set permission:', err);
      return null;
    }
  }

  /**
   * Get the current execution enablement state.
   * This is the execution policy layer the frontend extension should consult.
   */
  async getExecutionState(): Promise<ExecutionState> {
    if (!this.invoke) {
      return {
        trusted: false,
        execution_allowed: false,
        reason: 'not_in_desktop_app',
      };
    }

    try {
      const result = await this.invoke('get_execution_state', {
        origin: this.origin,
      });
      return result as ExecutionState;
    } catch (err) {
      console.error('[wiki3-desktop] Failed to get execution state:', err);
      return {
        trusted: false,
        execution_allowed: false,
        reason: 'execution_state_error',
      };
    }
  }
}
