/**
 * Commit & Push Panel.
 *
 * Shows git status for a workspace and allows committing and pushing changes.
 */

import * as api from '../api';
import type { GitStatus } from '../types';

export class CommitPushPanel {
  private container: HTMLElement | null = null;
  private workspaceId: string;
  private status: GitStatus | null = null;

  constructor(workspaceId: string) {
    this.workspaceId = workspaceId;
  }

  /** Render into a container and load initial status. */
  async render(container: HTMLElement): Promise<void> {
    this.container = container;
    await this.refresh();
  }

  /** Reload git status. */
  async refresh(): Promise<void> {
    if (!this.container) return;

    this.container.innerHTML = '<div class="w3-panel"><p>Loading status...</p></div>';

    try {
      this.status = await api.getGitStatus(this.workspaceId);
      this.renderStatus();
    } catch (err) {
      this.container.innerHTML = `
        <div class="w3-panel w3-error">
          <p>Failed to load status: ${err}</p>
        </div>
      `;
    }
  }

  private renderStatus(): void {
    if (!this.container || !this.status) return;

    const { branch, dirty_files, ahead, behind, last_commit } = this.status;

    const filesHtml =
      dirty_files.length > 0
        ? `<div class="w3-file-list">
            ${dirty_files
              .map(
                (f) =>
                  `<div class="w3-file-item">
                    <span class="w3-file-status w3-status-${f.status}">${f.status}</span>
                    <span class="w3-file-path">${f.path}</span>
                  </div>`,
              )
              .join('')}
          </div>`
        : '<p class="w3-muted">Working tree is clean — no changes to commit.</p>';

    const lastCommitHtml = last_commit
      ? `<div class="w3-last-commit">
          Last commit: <code>${last_commit.sha}</code> — ${last_commit.message}
          <br><small>${last_commit.author}, ${last_commit.date}</small>
        </div>`
      : '<p class="w3-muted">No commits yet.</p>';

    this.container.innerHTML = `
      <div class="w3-panel">
        <h3>Commit &amp; Push</h3>

        <div class="w3-status-bar">
          <span class="w3-ws-branch">${branch}</span>
          ${ahead > 0 ? `<span class="w3-ahead">↑${ahead}</span>` : ''}
          ${behind > 0 ? `<span class="w3-behind">↓${behind}</span>` : ''}
          <span class="w3-file-count">${dirty_files.length} changed file${dirty_files.length !== 1 ? 's' : ''}</span>
        </div>

        ${filesHtml}
        ${lastCommitHtml}

        <div class="w3-commit-form" style="${dirty_files.length === 0 ? 'display:none;' : ''}">
          <input type="text" id="w3-commit-msg" placeholder="Commit message" class="w3-input" />
          <div class="w3-dialog-actions">
            <button class="w3-btn" id="w3-commit-only">Commit</button>
            <button class="w3-btn w3-btn-primary" id="w3-commit-push">Commit &amp; Push</button>
          </div>
        </div>

        <div class="w3-dialog-actions" style="${ahead === 0 && dirty_files.length > 0 ? 'display:none;' : ''}">
          <button class="w3-btn" id="w3-push-only" ${ahead === 0 ? 'disabled' : ''}>
            Push ${ahead > 0 ? `(${ahead} commit${ahead !== 1 ? 's' : ''})` : ''}
          </button>
          <button class="w3-btn" id="w3-refresh">Refresh</button>
        </div>

        <div class="w3-dialog-status" id="w3-cp-status" style="display:none;"></div>
      </div>
    `;

    this.injectStyles();
    this.bindEvents();
  }

  private bindEvents(): void {
    if (!this.container) return;

    const commitMsg = this.container.querySelector('#w3-commit-msg') as HTMLInputElement;
    const statusEl = this.container.querySelector('#w3-cp-status') as HTMLElement;

    const showStatus = (msg: string, isError = false) => {
      statusEl.style.display = 'block';
      statusEl.textContent = msg;
      statusEl.className = `w3-dialog-status${isError ? ' w3-error' : ''}`;
    };

    this.container.querySelector('#w3-commit-only')?.addEventListener('click', async () => {
      const msg = commitMsg.value.trim();
      if (!msg) {
        showStatus('Please enter a commit message', true);
        return;
      }
      showStatus('Committing...');
      try {
        const info = await api.commitChanges(this.workspaceId, msg);
        showStatus(`Committed: ${info.sha} — ${info.message}`);
        await this.refresh();
      } catch (err) {
        showStatus(`Commit failed: ${err}`, true);
      }
    });

    this.container.querySelector('#w3-commit-push')?.addEventListener('click', async () => {
      const msg = commitMsg.value.trim();
      if (!msg) {
        showStatus('Please enter a commit message', true);
        return;
      }
      showStatus('Committing and pushing...');
      try {
        const result = await api.commitAndPush(this.workspaceId, msg);
        if (result.push.success) {
          showStatus(`Pushed: ${result.commit.sha} → ${result.push.remote}/${result.push.branch}`);
        } else {
          showStatus(`Committed but push failed: ${result.push.message}`, true);
        }
        await this.refresh();
      } catch (err) {
        showStatus(`Error: ${err}`, true);
      }
    });

    this.container.querySelector('#w3-push-only')?.addEventListener('click', async () => {
      showStatus('Pushing...');
      try {
        const result = await api.pushChanges(this.workspaceId);
        if (result.success) {
          showStatus(`Pushed to ${result.remote}/${result.branch}`);
        } else {
          showStatus(`Push failed: ${result.message}`, true);
        }
        await this.refresh();
      } catch (err) {
        showStatus(`Push failed: ${err}`, true);
      }
    });

    this.container.querySelector('#w3-refresh')?.addEventListener('click', () => this.refresh());
  }

  private injectStyles(): void {
    if (document.getElementById('w3-commit-push-styles')) return;
    const style = document.createElement('style');
    style.id = 'w3-commit-push-styles';
    style.textContent = `
      .w3-status-bar { display: flex; gap: 8px; align-items: center; margin-bottom: 12px; }
      .w3-ahead { color: #2e7d32; font-weight: 600; }
      .w3-behind { color: #c62828; font-weight: 600; }
      .w3-file-count { color: #666; font-size: 13px; }
      .w3-file-list { margin: 8px 0 16px; max-height: 200px; overflow-y: auto; border: 1px solid #e0e0e0; border-radius: 6px; }
      .w3-file-item { display: flex; gap: 8px; padding: 6px 10px; border-bottom: 1px solid #f0f0f0; font-size: 13px; }
      .w3-file-item:last-child { border-bottom: none; }
      .w3-file-status { font-size: 11px; text-transform: uppercase; padding: 1px 6px; border-radius: 3px; font-weight: 600; }
      .w3-status-modified { background: #fff3e0; color: #e65100; }
      .w3-status-added { background: #e8f5e9; color: #2e7d32; }
      .w3-status-deleted { background: #ffebee; color: #c62828; }
      .w3-status-untracked { background: #f3e5f5; color: #7b1fa2; }
      .w3-status-renamed { background: #e3f2fd; color: #1565c0; }
      .w3-file-path { font-family: monospace; }
      .w3-last-commit { font-size: 13px; color: #555; margin: 12px 0; padding: 8px; background: #f5f5f5; border-radius: 6px; }
      .w3-last-commit code { background: #e0e0e0; padding: 1px 4px; border-radius: 3px; }
      .w3-commit-form { margin-top: 16px; }
      .w3-input { width: 100%; padding: 8px 10px; border: 1px solid #d0d0d0; border-radius: 6px; font-size: 14px; margin-bottom: 8px; box-sizing: border-box; }
      .w3-input:focus { outline: none; border-color: #0066cc; box-shadow: 0 0 0 2px rgba(0,102,204,0.2); }
    `;
    document.head.appendChild(style);
  }
}
