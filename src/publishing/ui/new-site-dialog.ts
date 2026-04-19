/**
 * New Site from Template Dialog.
 *
 * Collects parameters and creates a new site from a template repository.
 */

import * as api from '../api';
import type { Workspace } from '../types';

export interface NewSiteDialogOptions {
  defaultTemplateOwner?: string;
  defaultTemplateRepo?: string;
}

export class NewSiteDialog {
  private dialog: HTMLDivElement | null = null;
  private options: NewSiteDialogOptions;

  constructor(options: NewSiteDialogOptions = {}) {
    this.options = {
      defaultTemplateOwner: options.defaultTemplateOwner ?? 'wiki3-ai',
      defaultTemplateRepo: options.defaultTemplateRepo ?? 'wiki3-ai-template',
    };
  }

  /** Show the dialog and return the created workspace, or null if cancelled. */
  async show(): Promise<Workspace | null> {
    return new Promise<Workspace | null>((resolve) => {
      this.dialog = document.createElement('div');
      this.dialog.className = 'w3-dialog-overlay';
      this.dialog.innerHTML = `
        <div class="w3-dialog">
          <h3>New Site from Template</h3>
          <form class="w3-form">
            <label>
              Owner (your GitHub username or org)
              <input type="text" name="owner" required placeholder="your-username" />
            </label>
            <label>
              Repository Name
              <input type="text" name="repoName" required placeholder="my-wiki3-site" />
            </label>
            <label>
              Description (optional)
              <input type="text" name="description" placeholder="My Wiki3 site" />
            </label>
            <label>
              Visibility
              <select name="visibility">
                <option value="public">Public</option>
                <option value="private">Private</option>
              </select>
            </label>
            <label>
              Template
              <input type="text" name="template" value="${this.options.defaultTemplateOwner}/${this.options.defaultTemplateRepo}" />
            </label>
            <div class="w3-dialog-actions">
              <button type="button" class="w3-btn" data-action="cancel">Cancel</button>
              <button type="submit" class="w3-btn w3-btn-primary">Create Site</button>
            </div>
            <div class="w3-dialog-status" style="display:none;"></div>
          </form>
        </div>
      `;

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
        const template = (data.get('template') as string).split('/');

        statusEl.style.display = 'block';
        statusEl.textContent = 'Creating repository...';
        statusEl.className = 'w3-dialog-status';

        try {
          const workspace = await api.createSiteFromTemplate({
            owner: data.get('owner') as string,
            repoName: data.get('repoName') as string,
            visibility: data.get('visibility') as 'public' | 'private',
            description: (data.get('description') as string) || undefined,
            templateOwner: template[0] || this.options.defaultTemplateOwner,
            templateRepo: template[1] || this.options.defaultTemplateRepo,
          });

          statusEl.textContent = `Created! Cloned to ${workspace.local_path}`;
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
      .w3-form input, .w3-form select { display: block; width: 100%; margin-top: 4px; padding: 8px 10px; border: 1px solid #d0d0d0; border-radius: 6px; font-size: 14px; box-sizing: border-box; }
      .w3-form input:focus, .w3-form select:focus { outline: none; border-color: #0066cc; box-shadow: 0 0 0 2px rgba(0,102,204,0.2); }
      .w3-dialog-actions { display: flex; gap: 8px; justify-content: flex-end; margin-top: 16px; }
      .w3-dialog-status { margin-top: 12px; padding: 8px 12px; border-radius: 6px; background: #e8f5e9; font-size: 13px; }
      .w3-dialog-status.w3-error { background: #ffebee; color: #c62828; }
    `;
    document.head.appendChild(style);
  }
}
