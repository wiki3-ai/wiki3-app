/**
 * Wiki3 Desktop Integration — Public API
 *
 * This module exports the desktop integration components for use by
 * the wiki3.ai JupyterLite site when running inside the desktop app.
 */

export { DesktopHostBridge } from './bridge';
export { Wiki3DesktopExtension } from './extension';
export { PermissionDialog } from './permission-dialog';
export {
  TOOL_NAMES,
  toolsStatus,
  toolsEnsure,
  toolsUninstall,
  toolsUninstallAll,
  toolsResolve,
  detectAppleContainer,
  onInstallProgress,
  onInstallDone,
} from './managed-tools';
export type {
  ToolName,
  ToolStatus,
  ToolStatusEntry,
  AppleContainerStatus,
  InstallProgress,
  InstallDonePayload,
} from './managed-tools';
export type {
  PermissionChoice,
  HostInfo,
  PermissionState,
  ExecutionState,
  AppConfig,
  DesktopIntegrationState,
} from './types';
