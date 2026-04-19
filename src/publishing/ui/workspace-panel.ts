/**
 * Workspace Panel — Main workspace management UI.
 *
 * Displays the current workspace state, git status, and provides
 * access to all publishing workflows.
 */

import * as api from '../api';
import type { GitStatus, Workspace } from '../types';

export class WorkspacePanel {
  private container: HTMLElement | null = null;
  private currentWorkspace: Workspace | null = null;
  private currentStatus: GitStatus | null = null;
  private onAction: ((action: string, data?: unknown) => void) | null = null;

  constructor() {}

  /** Set callback for user actions (navigate to sub-panels). */
  setActionHandler(handler: (action: string, data?: unknown) => void): void {
    this.onAction = handler;
  }

  /** Render the panel into a container element. */
  async render(container: HTMLElement): Promise<void> {
    this.container = container;
    this.injectStyles();
    await this.refresh();
  }

  /** Refresh workspace list and status. */
  async refresh(): Promise<void> {
    if (!this.container) return;

    try {
      const workspaces = await api.listWorkspaces();
      this.renderWorkspaceList(workspaces);
    } catch (err) {
      this.container.innerHTML = `
        <div class="w3-panel w3-error">
          <p>Failed to load workspaces: ${err}</p>
        </div>
      `;
    }
  }

  private renderWorkspaceList(workspaces: Workspace[]): void {
    if (!this.container) return;

    const actions = `
      <div class="w3-actions">
        <button class="w3-btn w3-btn-primary" data-action="new-from-template">
          New Site from Template
        </button>
        <button class="w3-btn" data-action="fork-site">
          Fork Existing Site
        </button>
        <button class="w3-btn" data-action="open-local">
          Open Local Workspace
        </button>
      </div>
    `;

    if (workspaces.length === 0) {
      this.container.innerHTML = `
        <div class="w3-panel">
          <h2>Workspaces</h2>
          <p class="w3-muted">No workspaces yet. Create one to get started.</p>
          ${actions}
        </div>
      `;
    } else {
      const items = workspaces
        .map(
          (ws) => `
        <div class="w3-workspace-card" data-workspace-id="${ws.id}">
          <div class="w3-ws-header">
            <strong>${ws.name}</strong>
            <span class="w3-ws-provider">${ws.provider}</span>
          </div>
          <div class="w3-ws-meta">
            <span>${ws.owner}/${ws.repo}</span>
            <span class="w3-ws-branch">${ws.branch}</span>
            <span class="w3-ws-mode">${ws.publish_mode.replace('_', ' ')}</span>
          </div>
          ${ws.site_url ? `<a class="w3-ws-url" href="${ws.site_url}" target="_blank">${ws.site_url}</a>` : ''}
          <div class="w3-ws-actions">
            <button class="w3-btn w3-btn-sm" data-action="status" data-ws="${ws.id}">Status</button>
            <button class="w3-btn w3-btn-sm" data-action="commit-push" data-ws="${ws.id}">Commit &amp; Push</button>
            <button class="w3-btn w3-btn-sm w3-btn-primary" data-action="publish" data-ws="${ws.id}">Publish</button>
            <button class="w3-btn w3-btn-sm w3-btn-danger" data-action="remove" data-ws="${ws.id}">Remove</button>
          </div>
        </div>
      `,
        )
        .join('');

      this.container.innerHTML = `
        <div class="w3-panel">
          <h2>Workspaces</h2>
          ${actions}
          <div class="w3-workspace-list">${items}</div>
        </div>
      `;
    }

    // Bind action buttons
    this.container.querySelectorAll('[data-action]').forEach((btn) => {
      btn.addEventListener('click', (e) => {
        const target = e.currentTarget as HTMLElement;
        const action = target.dataset.action;
        const wsId = target.dataset.ws;
        if (action && this.onAction) {
          this.onAction(action, wsId ? { workspaceId: wsId } : undefined);
        }
      });
    });
  }

  private injectStyles(): void {
    if (document.getElementById('w3-workspace-styles')) return;
    const style = document.createElement('style');
    style.id = 'w3-workspace-styles';
    style.textContent = `
      .w3-panel { padding: 20px; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; }
      .w3-panel h2 { margin: 0 0 16px; font-size: 20px; font-weight: 600; }
      .w3-muted { color: #666; }
      .w3-error { color: #cc3300; }
      .w3-actions { display: flex; gap: 8px; margin-bottom: 20px; flex-wrap: wrap; }
      .w3-btn { padding: 8px 16px; border: 1px solid #d0d0d0; border-radius: 6px; background: #fff; cursor: pointer; font-size: 13px; }
      .w3-btn:hover { background: #f0f0f0; }
      .w3-btn-primary { background: #0066cc; color: #fff; border-color: #0066cc; }
      .w3-btn-primary:hover { background: #0055aa; }
      .w3-btn-danger { color: #cc3300; border-color: #cc3300; }
      .w3-btn-danger:hover { background: #fff0f0; }
      .w3-btn-sm { padding: 4px 10px; font-size: 12px; }
      .w3-workspace-list { display: flex; flex-direction: column; gap: 12px; }
      .w3-workspace-card { padding: 16px; border: 1px solid #e0e0e0; border-radius: 8px; background: #fafafa; }
      .w3-ws-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px; }
      .w3-ws-provider { font-size: 11px; text-transform: uppercase; color: #888; background: #f0f0f0; padding: 2px 8px; border-radius: 4px; }
      .w3-ws-meta { display: flex; gap: 12px; font-size: 13px; color: #555; margin-bottom: 6px; }
      .w3-ws-branch { background: #e8f5e9; padding: 1px 6px; border-radius: 3px; color: #2e7d32; }
      .w3-ws-mode { background: #e3f2fd; padding: 1px 6px; border-radius: 3px; color: #1565c0; }
      .w3-ws-url { font-size: 12px; color: #0066cc; display: block; margin-bottom: 8px; }
      .w3-ws-actions { display: flex; gap: 6px; margin-top: 8px; }
    `;
    document.head.appendChild(style);
  }
}
