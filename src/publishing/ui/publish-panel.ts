/**
 * Publish Site Panel.
 *
 * Shows publish mode info and triggers site publication.
 */

import * as api from '../api';
import type { Workspace } from '../types';

export class PublishPanel {
  private container: HTMLElement | null = null;
  private workspace: Workspace;

  constructor(workspace: Workspace) {
    this.workspace = workspace;
  }

  /** Render into a container. */
  async render(container: HTMLElement): Promise<void> {
    this.container = container;
    this.renderContent();
  }

  private renderContent(): void {
    if (!this.container) return;

    const modeLabel = this.workspace.publish_mode === 'gh_pages_branch'
      ? 'gh-pages branch'
      : this.workspace.publish_mode === 'docs_folder'
        ? '/docs folder on main'
        : 'Not configured';

    this.container.innerHTML = `
      <div class="w3-panel">
        <h3>Publish Site</h3>

        <div class="w3-publish-info">
          <div class="w3-publish-row">
            <span class="w3-publish-label">Repository</span>
            <span>${this.workspace.owner}/${this.workspace.repo}</span>
          </div>
          <div class="w3-publish-row">
            <span class="w3-publish-label">Publish Mode</span>
            <span class="w3-ws-mode">${modeLabel}</span>
          </div>
          ${this.workspace.site_url ? `
            <div class="w3-publish-row">
              <span class="w3-publish-label">Site URL</span>
              <a href="${this.workspace.site_url}" target="_blank">${this.workspace.site_url}</a>
            </div>
          ` : ''}
        </div>

        <div class="w3-dialog-actions">
          <button class="w3-btn" id="w3-detect-mode">Detect Mode</button>
          <button class="w3-btn w3-btn-primary" id="w3-publish-btn">Publish / Update</button>
        </div>

        <div class="w3-dialog-status" id="w3-publish-status" style="display:none;"></div>
      </div>
    `;

    this.injectStyles();
    this.bindEvents();
  }

  private bindEvents(): void {
    if (!this.container) return;

    const statusEl = this.container.querySelector('#w3-publish-status') as HTMLElement;

    const showStatus = (msg: string, isError = false) => {
      statusEl.style.display = 'block';
      statusEl.textContent = msg;
      statusEl.className = `w3-dialog-status${isError ? ' w3-error' : ''}`;
    };

    this.container.querySelector('#w3-detect-mode')?.addEventListener('click', async () => {
      showStatus('Detecting publish mode...');
      try {
        const mode = await api.detectPublishMode(this.workspace.id);
        showStatus(`Detected mode: ${mode}`);
      } catch (err) {
        showStatus(`Detection failed: ${err}`, true);
      }
    });

    this.container.querySelector('#w3-publish-btn')?.addEventListener('click', async () => {
      showStatus('Publishing site...');
      try {
        const result = await api.publishSite(this.workspace.id);
        if (result.success) {
          let msg = `Published! Mode: ${result.publish_mode}`;
          if (result.site_url) {
            msg += `\nSite: ${result.site_url}`;
          }
          showStatus(msg);
        } else {
          showStatus(`Publish failed: ${result.message}`, true);
        }
      } catch (err) {
        showStatus(`Publish failed: ${err}`, true);
      }
    });
  }

  private injectStyles(): void {
    if (document.getElementById('w3-publish-styles')) return;
    const style = document.createElement('style');
    style.id = 'w3-publish-styles';
    style.textContent = `
      .w3-publish-info { margin-bottom: 16px; }
      .w3-publish-row { display: flex; gap: 12px; padding: 6px 0; border-bottom: 1px solid #f0f0f0; font-size: 13px; }
      .w3-publish-label { font-weight: 600; min-width: 100px; color: #555; }
      .w3-publish-row a { color: #0066cc; }
    `;
    document.head.appendChild(style);
  }
}
