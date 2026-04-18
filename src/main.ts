/**
 * Wiki3 Desktop App — Main Entry Point
 *
 * This is the initial page loaded by the Tauri webview.
 * The Tauri setup handler immediately navigates the window to wiki3.ai,
 * so this page is only shown briefly as a loading screen.
 *
 * When wiki3.ai loads, the desktop extension (injected as part of the
 * JupyterLite site build or via content script) initializes the
 * desktop integration bridge.
 */

console.log('[wiki3-app] Loading...');

// The Tauri backend navigates the main window to wiki3.ai during setup.
// This script only handles the brief loading state and any error display.

async function init(): Promise<void> {
  try {
    // Check if Tauri API is available
    const internals = (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    if (internals) {
      console.log('[wiki3-app] Tauri desktop host detected, waiting for navigation...');
    } else {
      console.log('[wiki3-app] Running outside Tauri (standalone mode).');
      showError('This page is intended to run inside the Wiki3 desktop app.');
    }
  } catch (err) {
    console.error('[wiki3-app] Initialization error:', err);
    showError('Failed to initialize. Please restart the app.');
  }
}

function showError(message: string): void {
  const el = document.getElementById('error-message');
  if (el) {
    el.textContent = message;
    el.style.display = 'block';
  }
}

init();
