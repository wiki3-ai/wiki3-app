/**
 * Permission Dialog for Desktop Execution Gating
 *
 * Provides a UI for requesting user permission before enabling
 * notebook/cell execution in the desktop app.
 */

import type { PermissionChoice } from './types';
import { Wiki3DesktopExtension } from './extension';

/**
 * Permission dialog that integrates into the JupyterLab/JupyterLite UI.
 */
export class PermissionDialog {
  private extension: Wiki3DesktopExtension;
  private dialogElement: HTMLElement | null = null;

  constructor(extension: Wiki3DesktopExtension) {
    this.extension = extension;
  }

  /**
   * Show the permission dialog if permission is needed.
   * Returns the user's choice, or null if no dialog was needed.
   */
  async showIfNeeded(): Promise<PermissionChoice | null> {
    if (!this.extension.needsPermission()) {
      return null;
    }

    return this.show();
  }

  /**
   * Show the permission dialog and wait for user response.
   */
  show(): Promise<PermissionChoice> {
    return new Promise((resolve) => {
      this.createDialog(resolve);
    });
  }

  /**
   * Remove the dialog from the DOM.
   */
  dismiss(): void {
    if (this.dialogElement?.parentNode) {
      this.dialogElement.parentNode.removeChild(this.dialogElement);
      this.dialogElement = null;
    }
  }

  private createDialog(onChoice: (choice: PermissionChoice) => void): void {
    // Remove any existing dialog
    this.dismiss();

    const overlay = document.createElement('div');
    overlay.className = 'wiki3-permission-overlay';
    overlay.setAttribute('role', 'dialog');
    overlay.setAttribute('aria-modal', 'true');
    overlay.setAttribute('aria-labelledby', 'wiki3-permission-title');

    overlay.innerHTML = `
      <div class="wiki3-permission-dialog">
        <h2 id="wiki3-permission-title" class="wiki3-permission-title">
          Allow Notebook Execution?
        </h2>
        <p class="wiki3-permission-message">
          This notebook wants to execute code using the JupyterLite kernel.
          This will run Python (Pyodide/WASM) or JavaScript code in your browser.
        </p>
        <div class="wiki3-permission-actions">
          <button class="wiki3-btn wiki3-btn-secondary" data-choice="deny">
            Deny
          </button>
          <button class="wiki3-btn wiki3-btn-secondary" data-choice="allow_once">
            Allow Once
          </button>
          <button class="wiki3-btn wiki3-btn-primary" data-choice="allow_always">
            Always Allow
          </button>
        </div>
        <p class="wiki3-permission-note">
          You can change this later in the desktop app settings.
        </p>
      </div>
    `;

    // Add styles
    this.injectStyles();

    // Wire up button handlers
    const buttons = overlay.querySelectorAll<HTMLButtonElement>('[data-choice]');
    buttons.forEach((button) => {
      button.addEventListener('click', async () => {
        const choice = button.dataset.choice as PermissionChoice;
        await this.extension.requestPermission(choice);
        this.dismiss();
        onChoice(choice);
      });
    });

    document.body.appendChild(overlay);
    this.dialogElement = overlay;

    // Focus the primary button
    const primaryBtn = overlay.querySelector<HTMLButtonElement>('.wiki3-btn-primary');
    primaryBtn?.focus();
  }

  private injectStyles(): void {
    if (document.getElementById('wiki3-permission-styles')) return;

    const style = document.createElement('style');
    style.id = 'wiki3-permission-styles';
    style.textContent = `
      .wiki3-permission-overlay {
        position: fixed;
        top: 0;
        left: 0;
        width: 100%;
        height: 100%;
        background: rgba(0, 0, 0, 0.5);
        display: flex;
        align-items: center;
        justify-content: center;
        z-index: 10000;
        font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
      }

      .wiki3-permission-dialog {
        background: white;
        border-radius: 12px;
        padding: 24px 32px;
        max-width: 460px;
        width: 90%;
        box-shadow: 0 20px 60px rgba(0, 0, 0, 0.3);
      }

      .wiki3-permission-title {
        margin: 0 0 12px 0;
        font-size: 18px;
        font-weight: 600;
        color: #1a1a2e;
      }

      .wiki3-permission-message {
        margin: 0 0 20px 0;
        font-size: 14px;
        line-height: 1.5;
        color: #444;
      }

      .wiki3-permission-actions {
        display: flex;
        gap: 8px;
        justify-content: flex-end;
      }

      .wiki3-btn {
        padding: 8px 16px;
        border-radius: 6px;
        border: 1px solid #ccc;
        font-size: 14px;
        font-weight: 500;
        cursor: pointer;
        transition: background 0.15s;
      }

      .wiki3-btn-secondary {
        background: #f5f5f5;
        color: #333;
      }

      .wiki3-btn-secondary:hover {
        background: #e8e8e8;
      }

      .wiki3-btn-primary {
        background: #0066cc;
        color: white;
        border-color: #0066cc;
      }

      .wiki3-btn-primary:hover {
        background: #0055aa;
      }

      .wiki3-permission-note {
        margin: 16px 0 0 0;
        font-size: 12px;
        color: #888;
      }
    `;
    document.head.appendChild(style);
  }
}
