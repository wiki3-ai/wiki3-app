/**
 * Wiki3 Desktop App — Main Entry Point
 *
 * Dashboard page that stays in the main window. Shows a repo URL input
 * (prepopulated with the saved default) so the user can open a GitHub
 * repo's published site in a new window.
 */

import * as api from './publishing/api';

async function init(): Promise<void> {
  console.log('[wiki3-app] Dashboard loading...');

  const repoInput = document.getElementById('repo-url-input') as HTMLInputElement | null;
  const openBtn = document.getElementById('open-repo-btn') as HTMLButtonElement | null;
  const mainContent = document.getElementById('main-content');

  if (!repoInput || !openBtn) return;

  // Load saved settings and apply the default repo URL
  try {
    const settings = await api.getSettings();
    if (settings.default_repo_url) {
      repoInput.value = settings.default_repo_url;
    }
  } catch {
    // Settings not available — keep the hardcoded default from HTML
  }

  async function handleOpenRepo(): Promise<void> {
    const url = repoInput!.value.trim();
    if (!url) return;

    openBtn!.disabled = true;
    openBtn!.textContent = 'Opening...';

    try {
      const result = await api.openRepoSite(url);
      if (mainContent) {
        mainContent.innerHTML = `<div style="text-align:center;padding:40px;color:#666;">
          <p>Opened <strong>${result.owner}/${result.repo}</strong></p>
          <p style="font-size:13px;">${result.site_url}</p>
        </div>`;
      }
      openBtn!.disabled = false;
      openBtn!.textContent = 'Open Site';
    } catch (err) {
      if (mainContent) {
        mainContent.innerHTML = `<div style="text-align:center;padding:40px;">
          <p style="color:#cc3300;">Failed to open: ${err}</p>
        </div>`;
      }
      openBtn!.disabled = false;
      openBtn!.textContent = 'Open Site';
    }
  }

  openBtn.addEventListener('click', handleOpenRepo);
  repoInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') handleOpenRepo();
  });
}

init();
