/**
 * Fork Site Dialog.
 *
 * Collects source repo info and forks it for the authenticated user.
 */

import * as api from '../api';
import type { Workspace } from '../types';

export class ForkSiteDialog {
  private dialog: HTMLDivElement | null = null;

  /** Show the dialog and return the created workspace, or null if cancelled. */
  async show(): Promise<Workspace | null> {
    return new Promise<Workspace | null>((resolve) => {
      this.dialog = document.createElement('div');
      this.dialog.className = 'w3-dialog-overlay';
      this.dialog.innerHTML = `
        <div class="w3-dialog">
          <h3>Fork Existing Site</h3>
          <form class="w3-form">
            <label>
              Source Repository (owner/repo)
              <input type="text" name="sourceRepo" required placeholder="wiki3-ai/wiki3-ai-site" />
            </label>
            <label>
              Fork Name (optional, defaults to source name)
              <input type="text" name="forkName" placeholder="" />
            </label>
            <label>
              Fork To (optional, org name; defaults to your user)
              <input type="text" name="targetOwner" placeholder="" />
            </label>
            <div class="w3-dialog-actions">
              <button type="button" class="w3-btn" data-action="cancel">Cancel</button>
              <button type="submit" class="w3-btn w3-btn-primary">Fork &amp; Clone</button>
            </div>
            <div class="w3-dialog-status" style="display:none;"></div>
          </form>
        </div>
      `;

      // Reuse the dialog styles from new-site-dialog
      this.injectStyles();
      document.body.appendChild(this.dialog);

      const form = this.dialog.querySelector('form') as HTMLFormElement;
      const statusEl = this.dialog.querySelector('.w3-dialog-status') as HTMLElement;

      this.dialog.querySelector('[data-action="cancel"]')?.addEventListener('click', () => {
        this.dismiss();
        resolve(null);
      });

      form.addEventListener('submit', async (e) => {
        e.preventDefault();
        const data = new FormData(form);
        const sourceRepo = (data.get('sourceRepo') as string).split('/');

        if (sourceRepo.length < 2) {
          statusEl.style.display = 'block';
          statusEl.textContent = 'Please enter owner/repo format';
          statusEl.className = 'w3-dialog-status w3-error';
          return;
        }

        statusEl.style.display = 'block';
        statusEl.textContent = 'Forking repository (this may take a moment)...';
        statusEl.className = 'w3-dialog-status';

        try {
          const workspace = await api.forkSite({
            sourceOwner: sourceRepo[0],
            sourceRepo: sourceRepo[1],
            forkName: (data.get('forkName') as string) || undefined,
            targetOwner: (data.get('targetOwner') as string) || undefined,
          });

          statusEl.textContent = `Forked! Cloned to ${workspace.local_path}`;
          setTimeout(() => {
            this.dismiss();
            resolve(workspace);
          }, 1500);
        } catch (err) {
          statusEl.textContent = `Error: ${err}`;
          statusEl.className = 'w3-dialog-status w3-error';
        }
      });
    });
  }

  dismiss(): void {
    this.dialog?.remove();
    this.dialog = null;
  }

  private injectStyles(): void {
    if (document.getElementById('w3-dialog-styles')) return;
    const style = document.createElement('style');
    style.id = 'w3-dialog-styles';
    style.textContent = `
      .w3-dialog-overlay { position: fixed; top: 0; left: 0; right: 0; bottom: 0; background: rgba(0,0,0,0.4); display: flex; align-items: center; justify-content: center; z-index: 10000; }
      .w3-dialog { background: #fff; border-radius: 12px; padding: 24px; width: 420px; max-width: 90vw; box-shadow: 0 8px 32px rgba(0,0,0,0.2); font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; }
      .w3-dialog h3 { margin: 0 0 16px; font-size: 18px; }
      .w3-form label { display: block; margin-bottom: 12px; font-size: 13px; color: #333; }
      .w3-form input { display: block; width: 100%; margin-top: 4px; padding: 8px 10px; border: 1px solid #d0d0d0; border-radius: 6px; font-size: 14px; box-sizing: border-box; }
      .w3-dialog-actions { display: flex; gap: 8px; justify-content: flex-end; margin-top: 16px; }
      .w3-dialog-status { margin-top: 12px; padding: 8px 12px; border-radius: 6px; background: #e8f5e9; font-size: 13px; }
      .w3-dialog-status.w3-error { background: #ffebee; color: #c62828; }
    `;
    document.head.appendChild(style);
  }
}
