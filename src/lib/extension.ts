/**
 * Wiki3 Desktop Extension
 *
 * JupyterLab/JupyterLite frontend extension for desktop integration.
 *
 * This extension:
 * - Detects when wiki3.ai is running inside the Wiki3 desktop app
 * - Registers desktop integration behavior only when the desktop host is available
 * - Integrates desktop-specific behavior (permission gating, execution policy)
 * - Keeps the site functioning normally when opened outside the desktop app
 */

import { DesktopHostBridge } from './bridge';
import type {
  DesktopIntegrationState,
  PermissionChoice,
  ExecutionState,
} from './types';

/**
 * The desktop extension manages the lifecycle of desktop integration.
 */
export class Wiki3DesktopExtension {
  private bridge: DesktopHostBridge;
  private state: DesktopIntegrationState;
  private listeners: Array<(state: DesktopIntegrationState) => void> = [];

  constructor() {
    this.bridge = new DesktopHostBridge();
    this.state = {
      isDesktop: false,
      hostInfo: null,
      permissionState: null,
      executionState: null,
      initialized: false,
    };
  }

  /**
   * Initialize the desktop extension.
   * Should be called once when the JupyterLab/JupyterLite app starts.
   */
  async initialize(): Promise<DesktopIntegrationState> {
    console.log('[wiki3-desktop] Initializing desktop extension...');

    // Step 1: Detect desktop host
    const hostInfo = await this.bridge.detectHost();
    this.state.hostInfo = hostInfo;
    this.state.isDesktop = hostInfo.detected;

    if (!hostInfo.detected) {
      console.log('[wiki3-desktop] Not running in desktop app, skipping desktop integration.');
      this.state.initialized = true;
      this.notifyListeners();
      return this.state;
    }

    console.log('[wiki3-desktop] Desktop host detected:', hostInfo);

    // Step 2: Get current permission state
    this.state.permissionState = await this.bridge.getPermissionState();

    // Step 3: Get execution state
    this.state.executionState = await this.bridge.getExecutionState();

    this.state.initialized = true;
    this.notifyListeners();

    console.log('[wiki3-desktop] Desktop extension initialized:', this.state);
    return this.state;
  }

  /**
   * Request execution permission from the user.
   * This should be called when the user attempts to run a notebook/cell.
   */
  async requestPermission(choice: PermissionChoice): Promise<void> {
    if (!this.state.isDesktop) {
      console.warn('[wiki3-desktop] Cannot request permission outside desktop app.');
      return;
    }

    const result = await this.bridge.setPermission(choice);
    if (result) {
      this.state.permissionState = result;
    }

    // Refresh execution state after permission change
    this.state.executionState = await this.bridge.getExecutionState();
    this.notifyListeners();
  }

  /**
   * Check whether execution is currently allowed.
   * The frontend should consult this before enabling Run behavior.
   */
  isExecutionAllowed(): boolean {
    if (!this.state.isDesktop) {
      // When not in desktop app, defer to the site's own behavior
      return true;
    }
    return this.state.executionState?.execution_allowed ?? false;
  }

  /**
   * Check whether we need to prompt for permission.
   */
  needsPermission(): boolean {
    if (!this.state.isDesktop) return false;
    return this.state.executionState?.needs_permission ?? true;
  }

  /**
   * Get the current integration state.
   */
  getState(): Readonly<DesktopIntegrationState> {
    return this.state;
  }

  /**
   * Subscribe to state changes.
   */
  onStateChange(listener: (state: DesktopIntegrationState) => void): () => void {
    this.listeners.push(listener);
    return () => {
      this.listeners = this.listeners.filter((l) => l !== listener);
    };
  }

  /**
   * Refresh the execution state from the desktop host.
   */
  async refreshExecutionState(): Promise<ExecutionState> {
    const executionState = await this.bridge.getExecutionState();
    this.state.executionState = executionState;
    this.notifyListeners();
    return executionState;
  }

  private notifyListeners(): void {
    const snapshot = { ...this.state };
    for (const listener of this.listeners) {
      try {
        listener(snapshot);
      } catch (err) {
        console.error('[wiki3-desktop] Listener error:', err);
      }
    }
  }
}
