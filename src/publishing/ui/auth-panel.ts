/**
 * Auth Panel.
 *
 * Handles GitHub token setup and displays auth status.
 */

import * as api from '../api';

export class AuthPanel {
  private container: HTMLElement | null = null;

  /** Render the auth panel into a container. */
  async render(container: HTMLElement): Promise<void> {
    this.container = container;
    await this.refresh();
  }

  /** Refresh auth status display. */
  async refresh(): Promise<void> {
    if (!this.container) return;

    try {
      const status = await api.getAuthStatus();
      if (status.authenticated && status.user) {
        this.renderAuthenticated(status.user.login, status.user.name ?? undefined);
      } else {
        this.renderUnauthenticated(status.reason);
      }
    } catch {
      this.renderUnauthenticated();
    }
  }

  private renderAuthenticated(login: string, name?: string): void {
    if (!this.container) return;

    this.container.innerHTML = `
      <div class="w3-auth-status">
        <span class="w3-auth-badge w3-auth-ok">✓ Authenticated</span>
        <span>${name ?? login} (${login})</span>
        <button class="w3-btn w3-btn-sm w3-btn-danger" id="w3-sign-out">Sign Out</button>
      </div>
    `;

    this.container.querySelector('#w3-sign-out')?.addEventListener('click', async () => {
      await api.clearGitHubAuth();
      await this.refresh();
    });
  }

  private renderUnauthenticated(reason?: string): void {
    if (!this.container) return;

    this.container.innerHTML = `
      <div class="w3-auth-form">
        <span class="w3-auth-badge w3-auth-none">Not authenticated</span>
        ${reason ? `<p class="w3-muted">${reason}</p>` : ''}
        <p class="w3-muted">Enter a GitHub Personal Access Token with <code>repo</code> scope.</p>
        <div style="display:flex; gap: 8px;">
          <input type="password" id="w3-token-input" placeholder="ghp_..." class="w3-input" style="flex:1;" />
          <button class="w3-btn w3-btn-primary" id="w3-save-token">Authenticate</button>
        </div>
        <div class="w3-dialog-status" id="w3-auth-status" style="display:none;"></div>
      </div>
    `;

    this.injectStyles();

    this.container.querySelector('#w3-save-token')?.addEventListener('click', async () => {
      const input = this.container?.querySelector('#w3-token-input') as HTMLInputElement;
      const statusEl = this.container?.querySelector('#w3-auth-status') as HTMLElement;
      const token = input.value.trim();

      if (!token) {
        statusEl.style.display = 'block';
        statusEl.textContent = 'Please enter a token';
        statusEl.className = 'w3-dialog-status w3-error';
        return;
      }

      statusEl.style.display = 'block';
      statusEl.textContent = 'Validating token...';
      statusEl.className = 'w3-dialog-status';

      try {
        const result = await api.storeGitHubToken(token);
        if (result.authenticated) {
          statusEl.textContent = `Authenticated as ${result.user?.login}`;
          setTimeout(() => this.refresh(), 1000);
        } else {
          statusEl.textContent = 'Token validation failed';
          statusEl.className = 'w3-dialog-status w3-error';
        }
      } catch (err) {
        statusEl.textContent = `Error: ${err}`;
        statusEl.className = 'w3-dialog-status w3-error';
      }
    });
  }

  private injectStyles(): void {
    if (document.getElementById('w3-auth-styles')) return;
    const style = document.createElement('style');
    style.id = 'w3-auth-styles';
    style.textContent = `
      .w3-auth-status, .w3-auth-form { display: flex; flex-direction: column; gap: 8px; padding: 12px; border: 1px solid #e0e0e0; border-radius: 8px; background: #fafafa; }
      .w3-auth-badge { font-size: 12px; font-weight: 600; padding: 2px 8px; border-radius: 4px; width: fit-content; }
      .w3-auth-ok { background: #e8f5e9; color: #2e7d32; }
      .w3-auth-none { background: #fff3e0; color: #e65100; }
      .w3-auth-form code { background: #e0e0e0; padding: 1px 4px; border-radius: 3px; }
    `;
    document.head.appendChild(style);
  }
}
