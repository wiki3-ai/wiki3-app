/**
 * Wiki3 Desktop App — Main Entry Point
 *
 * This is the app's dashboard controller. It manages the top-level navigation
 * and mounts the publishing workflow panels into the main content area.
 *
 * Views:
 *   - workspaces: lists workspaces with create/fork/open actions
 *   - auth: GitHub authentication setup
 *   - workspace-detail: git status, commit/push, publish for a single workspace
 */

import * as api from './publishing/api';
import type { AuthStatus, GitStatus, Workspace, PublishResult } from './publishing/types';

// ─── State ─────────────────────────────────────────────────────────────────

type View =
  | { name: 'workspaces' }
  | { name: 'auth' }
  | { name: 'workspace-detail'; workspaceId: string }
  | { name: 'new-from-template' }
  | { name: 'fork-site' };

let currentView: View = { name: 'workspaces' };
let authStatus: AuthStatus = { authenticated: false };

const mainContent = () => document.getElementById('main-content')!;
const authIndicator = () => document.getElementById('auth-indicator')!;

// ─── Init ──────────────────────────────────────────────────────────────────

async function init(): Promise<void> {
  console.log('[wiki3-app] Dashboard initializing...');

  // Wire up top nav
  document.querySelectorAll('#main-nav button').forEach((btn) => {
    btn.addEventListener('click', () => {
      const view = (btn as HTMLElement).dataset.view;
      if (view === 'workspaces') navigateTo({ name: 'workspaces' });
      else if (view === 'auth') navigateTo({ name: 'auth' });
    });
  });

  // Load auth status, then show the workspaces view
  await refreshAuthStatus();
  await navigateTo({ name: 'workspaces' });
}

// ─── Navigation ────────────────────────────────────────────────────────────

async function navigateTo(view: View): Promise<void> {
  currentView = view;
  updateNavHighlight();

  switch (view.name) {
    case 'workspaces':
      await renderWorkspaces();
      break;
    case 'auth':
      await renderAuth();
      break;
    case 'workspace-detail':
      await renderWorkspaceDetail(view.workspaceId);
      break;
    case 'new-from-template':
      renderNewFromTemplate();
      break;
    case 'fork-site':
      renderForkSite();
      break;
  }
}

function updateNavHighlight(): void {
  document.querySelectorAll('#main-nav button').forEach((btn) => {
    const view = (btn as HTMLElement).dataset.view;
    btn.classList.toggle('active', view === currentView.name || (view === 'workspaces' && currentView.name === 'workspace-detail'));
  });
}

// ─── Auth indicator ────────────────────────────────────────────────────────

async function refreshAuthStatus(): Promise<void> {
  try {
    authStatus = await api.getAuthStatus();
  } catch {
    authStatus = { authenticated: false };
  }
  renderAuthIndicator();
}

function renderAuthIndicator(): void {
  const el = authIndicator();
  if (authStatus.authenticated && authStatus.user) {
    el.innerHTML = `
      <span class="w3-auth-dot connected"></span>
      <span>${authStatus.user.login}</span>
    `;
  } else {
    el.innerHTML = `
      <span class="w3-auth-dot disconnected"></span>
      <span>Not connected</span>
    `;
  }
}

// ─── Workspaces View ───────────────────────────────────────────────────────

async function renderWorkspaces(): Promise<void> {
  const el = mainContent();
  el.innerHTML = '<div class="w3-loading"><div class="spinner"></div><p>Loading workspaces...</p></div>';

  let workspaces: Workspace[];
  try {
    workspaces = await api.listWorkspaces();
  } catch (err) {
    el.innerHTML = `<div class="w3-panel"><p class="w3-error">Failed to load workspaces: ${err}</p></div>`;
    return;
  }

  if (workspaces.length === 0) {
    el.innerHTML = `
      <div class="w3-empty">
        <h2>Welcome to Wiki3</h2>
        <p>Create a site from a template, fork an existing one, or open a local workspace.</p>
        <div class="w3-actions" style="justify-content:center;">
          <button class="w3-btn w3-btn-primary" data-action="new-from-template">New Site from Template</button>
          <button class="w3-btn" data-action="fork-site">Fork Existing Site</button>
          <button class="w3-btn" data-action="open-local">Open Local Workspace</button>
        </div>
        ${!authStatus.authenticated ? '<p class="w3-muted" style="margin-top:16px;font-size:13px;">💡 <a href="#" data-action="go-auth" style="color:#0066cc;">Connect your GitHub account</a> first to create or fork repos.</p>' : ''}
      </div>
    `;
  } else {
    const cards = workspaces.map((ws) => workspaceCard(ws)).join('');
    el.innerHTML = `
      <div class="w3-panel">
        <div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:16px;">
          <h2 style="margin:0;">Workspaces</h2>
        </div>
        <div class="w3-actions">
          <button class="w3-btn w3-btn-primary" data-action="new-from-template">New Site from Template</button>
          <button class="w3-btn" data-action="fork-site">Fork Existing Site</button>
          <button class="w3-btn" data-action="open-local">Open Local Workspace</button>
        </div>
        <div class="w3-workspace-list">${cards}</div>
      </div>
    `;
  }

  bindWorkspaceActions(el);
}

function workspaceCard(ws: Workspace): string {
  const modeLabel = ws.publish_mode === 'gh_pages_branch' ? 'gh-pages'
    : ws.publish_mode === 'docs_folder' ? '/docs' : 'none';

  return `
    <div class="w3-workspace-card" data-workspace-id="${ws.id}">
      <div class="w3-ws-header">
        <strong>${ws.owner}/${ws.repo}</strong>
        <span class="w3-ws-provider">${ws.provider}</span>
      </div>
      <div class="w3-ws-meta">
        <span class="w3-ws-branch">${ws.branch}</span>
        <span class="w3-ws-mode">${modeLabel}</span>
      </div>
      <span class="w3-ws-path">${ws.local_path}</span>
      ${ws.site_url ? `<a class="w3-ws-url" href="#" data-site-url="${ws.site_url}">${ws.site_url}</a>` : ''}
      <div class="w3-ws-actions">
        <button class="w3-btn w3-btn-sm" data-action="open-detail" data-ws="${ws.id}">Open</button>
        <button class="w3-btn w3-btn-sm w3-btn-danger" data-action="remove" data-ws="${ws.id}">Remove</button>
      </div>
    </div>
  `;
}

function bindWorkspaceActions(el: HTMLElement): void {
  el.querySelectorAll('[data-action]').forEach((btn) => {
    btn.addEventListener('click', async (e) => {
      e.preventDefault();
      const target = e.currentTarget as HTMLElement;
      const action = target.dataset.action;
      const wsId = target.dataset.ws;

      switch (action) {
        case 'new-from-template':
          if (!authStatus.authenticated) {
            navigateTo({ name: 'auth' });
          } else {
            navigateTo({ name: 'new-from-template' });
          }
          break;
        case 'fork-site':
          if (!authStatus.authenticated) {
            navigateTo({ name: 'auth' });
          } else {
            navigateTo({ name: 'fork-site' });
          }
          break;
        case 'open-local':
          await handleOpenLocal();
          break;
        case 'open-detail':
          if (wsId) navigateTo({ name: 'workspace-detail', workspaceId: wsId });
          break;
        case 'remove':
          if (wsId && confirm('Remove this workspace from the list? (Files will not be deleted.)')) {
            try {
              await api.removeWorkspace(wsId);
              await renderWorkspaces();
            } catch (err) {
              alert(`Failed to remove: ${err}`);
            }
          }
          break;
        case 'go-auth':
          navigateTo({ name: 'auth' });
          break;
      }
    });
  });

  // Site URL links → open in new Tauri window
  el.querySelectorAll('[data-site-url]').forEach((link) => {
    link.addEventListener('click', (e) => {
      e.preventDefault();
      const url = (e.currentTarget as HTMLElement).dataset.siteUrl;
      if (url) openSiteWindow(url);
    });
  });
}

async function handleOpenLocal(): Promise<void> {
  try {
    // Use Tauri dialog to pick a folder
    const { open } = await import('@tauri-apps/plugin-dialog');
    const selected = await open({ directory: true, title: 'Select a git repository folder' });
    if (!selected) return;

    const path = typeof selected === 'string' ? selected : String(selected);
    mainContent().innerHTML = '<div class="w3-loading"><div class="spinner"></div><p>Registering workspace...</p></div>';
    await api.openLocalWorkspace(path);
    await renderWorkspaces();
  } catch (err) {
    alert(`Failed to open workspace: ${err}`);
    await renderWorkspaces();
  }
}

function openSiteWindow(url: string): void {
  try {
    const invoke = (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ as
      | { invoke?: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> }
      | undefined;
    if (invoke?.invoke) {
      invoke.invoke('open_new_window', { url });
    } else {
      window.open(url, '_blank');
    }
  } catch {
    window.open(url, '_blank');
  }
}

// ─── Auth View ─────────────────────────────────────────────────────────────

async function renderAuth(): Promise<void> {
  const el = mainContent();
  await refreshAuthStatus();

  if (authStatus.authenticated && authStatus.user) {
    el.innerHTML = `
      <div class="w3-panel">
        <h2>GitHub Account</h2>
        <div class="w3-auth-section">
          <span class="w3-auth-badge w3-auth-ok">✓ Connected</span>
          <p style="margin:8px 0;font-size:14px;"><strong>${authStatus.user.name ?? authStatus.user.login}</strong> (${authStatus.user.login})</p>
          <button class="w3-btn w3-btn-danger w3-btn-sm" id="sign-out-btn">Disconnect</button>
        </div>
      </div>
    `;
    el.querySelector('#sign-out-btn')?.addEventListener('click', async () => {
      await api.clearGitHubAuth();
      await refreshAuthStatus();
      await renderAuth();
    });
  } else {
    el.innerHTML = `
      <div class="w3-panel">
        <h2>Connect GitHub</h2>
        <div class="w3-auth-section">
          <span class="w3-auth-badge w3-auth-none">Not connected</span>
          <p style="margin:8px 0;font-size:14px;">Enter a GitHub Personal Access Token with <code style="background:#e0e0e0;padding:1px 4px;border-radius:3px;">repo</code> scope to create, fork, and publish sites.</p>
          <p style="margin:4px 0 12px;font-size:12px;color:#888;">Create a token at <a href="#" id="pat-link" style="color:#0066cc;">github.com/settings/tokens</a></p>
          <div style="display:flex;gap:8px;">
            <input type="password" id="token-input" placeholder="ghp_..." style="flex:1;padding:8px 10px;border:1px solid #d0d0d0;border-radius:6px;font-size:14px;" />
            <button class="w3-btn w3-btn-primary" id="save-token-btn">Connect</button>
          </div>
          <div class="w3-dialog-status" id="auth-status-msg" style="display:none;"></div>
        </div>
      </div>
    `;

    el.querySelector('#pat-link')?.addEventListener('click', (e) => {
      e.preventDefault();
      openSiteWindow('https://github.com/settings/tokens');
    });

    el.querySelector('#save-token-btn')?.addEventListener('click', async () => {
      const input = el.querySelector('#token-input') as HTMLInputElement;
      const statusEl = el.querySelector('#auth-status-msg') as HTMLElement;
      const token = input.value.trim();

      if (!token) {
        showStatus(statusEl, 'Please enter a token.', true);
        return;
      }

      showStatus(statusEl, 'Validating token...');

      try {
        const result = await api.storeGitHubToken(token);
        if (result.authenticated) {
          showStatus(statusEl, `Connected as ${result.user?.login}!`);
          await refreshAuthStatus();
          setTimeout(() => renderAuth(), 1000);
        } else {
          showStatus(statusEl, 'Token validation failed.', true);
        }
      } catch (err) {
        showStatus(statusEl, `Error: ${err}`, true);
      }
    });
  }
}

// ─── New Site from Template ────────────────────────────────────────────────

function renderNewFromTemplate(): void {
  const el = mainContent();
  el.innerHTML = `
    <div class="w3-panel">
      <button class="w3-back" data-action="back">← Back to Workspaces</button>
      <h2>New Site from Template</h2>
      <form id="template-form" class="w3-form" style="max-width:500px;">
        <label>
          Owner (your GitHub username or org)
          <input type="text" name="owner" required placeholder="${authStatus.user?.login ?? 'your-username'}" value="${authStatus.user?.login ?? ''}" />
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
          Template Repository
          <input type="text" name="template" value="wiki3-ai/wiki3-ai-template" />
        </label>
        <div class="w3-dialog-actions" style="justify-content:flex-start;">
          <button type="submit" class="w3-btn w3-btn-primary">Create Site</button>
          <button type="button" class="w3-btn" data-action="back">Cancel</button>
        </div>
        <div class="w3-dialog-status" id="template-status" style="display:none;"></div>
      </form>
    </div>
  `;

  el.querySelectorAll('[data-action="back"]').forEach((btn) =>
    btn.addEventListener('click', () => navigateTo({ name: 'workspaces' })),
  );

  const form = el.querySelector('#template-form') as HTMLFormElement;
  form.addEventListener('submit', async (e) => {
    e.preventDefault();
    const data = new FormData(form);
    const statusEl = el.querySelector('#template-status') as HTMLElement;
    const submitBtn = form.querySelector('[type="submit"]') as HTMLButtonElement;

    const template = (data.get('template') as string).split('/');
    submitBtn.disabled = true;
    showStatus(statusEl, 'Creating repository and cloning...');

    try {
      const workspace = await api.createSiteFromTemplate({
        owner: data.get('owner') as string,
        repoName: data.get('repoName') as string,
        visibility: data.get('visibility') as 'public' | 'private',
        description: (data.get('description') as string) || undefined,
        templateOwner: template[0] || 'wiki3-ai',
        templateRepo: template[1] || 'wiki3-ai-template',
      });
      showStatus(statusEl, `✓ Created! Cloned to ${workspace.local_path}`);
      setTimeout(() => navigateTo({ name: 'workspace-detail', workspaceId: workspace.id }), 1500);
    } catch (err) {
      showStatus(statusEl, `Error: ${err}`, true);
      submitBtn.disabled = false;
    }
  });
}

// ─── Fork Site ─────────────────────────────────────────────────────────────

function renderForkSite(): void {
  const el = mainContent();
  el.innerHTML = `
    <div class="w3-panel">
      <button class="w3-back" data-action="back">← Back to Workspaces</button>
      <h2>Fork Existing Site</h2>
      <form id="fork-form" class="w3-form" style="max-width:500px;">
        <label>
          Source Repository (owner/repo)
          <input type="text" name="sourceRepo" required placeholder="wiki3-ai/wiki3-ai-site" />
        </label>
        <label>
          Fork Name (optional, defaults to source name)
          <input type="text" name="forkName" placeholder="" />
        </label>
        <label>
          Fork To (optional, org name or leave blank for your user)
          <input type="text" name="targetOwner" placeholder="" />
        </label>
        <div class="w3-dialog-actions" style="justify-content:flex-start;">
          <button type="submit" class="w3-btn w3-btn-primary">Fork &amp; Clone</button>
          <button type="button" class="w3-btn" data-action="back">Cancel</button>
        </div>
        <div class="w3-dialog-status" id="fork-status" style="display:none;"></div>
      </form>
    </div>
  `;

  el.querySelectorAll('[data-action="back"]').forEach((btn) =>
    btn.addEventListener('click', () => navigateTo({ name: 'workspaces' })),
  );

  const form = el.querySelector('#fork-form') as HTMLFormElement;
  form.addEventListener('submit', async (e) => {
    e.preventDefault();
    const data = new FormData(form);
    const statusEl = el.querySelector('#fork-status') as HTMLElement;
    const submitBtn = form.querySelector('[type="submit"]') as HTMLButtonElement;

    const sourceRepo = (data.get('sourceRepo') as string).split('/');
    if (sourceRepo.length < 2 || !sourceRepo[0] || !sourceRepo[1]) {
      showStatus(statusEl, 'Please enter in owner/repo format.', true);
      return;
    }

    submitBtn.disabled = true;
    showStatus(statusEl, 'Forking repository (this may take a moment)...');

    try {
      const workspace = await api.forkSite({
        sourceOwner: sourceRepo[0],
        sourceRepo: sourceRepo[1],
        forkName: (data.get('forkName') as string) || undefined,
        targetOwner: (data.get('targetOwner') as string) || undefined,
      });
      showStatus(statusEl, `✓ Forked! Cloned to ${workspace.local_path}`);
      setTimeout(() => navigateTo({ name: 'workspace-detail', workspaceId: workspace.id }), 1500);
    } catch (err) {
      showStatus(statusEl, `Error: ${err}`, true);
      submitBtn.disabled = false;
    }
  });
}

// ─── Workspace Detail View ─────────────────────────────────────────────────

async function renderWorkspaceDetail(workspaceId: string): Promise<void> {
  const el = mainContent();
  el.innerHTML = '<div class="w3-loading"><div class="spinner"></div><p>Loading workspace...</p></div>';

  let workspace: Workspace | null;
  try {
    workspace = await api.getWorkspace(workspaceId);
  } catch (err) {
    el.innerHTML = `<div class="w3-panel"><p class="w3-error">Failed to load workspace: ${err}</p></div>`;
    return;
  }

  if (!workspace) {
    el.innerHTML = `<div class="w3-panel"><p class="w3-error">Workspace not found.</p></div>`;
    return;
  }

  let status: GitStatus | null = null;
  try {
    status = await api.getGitStatus(workspaceId);
  } catch {
    // git status might fail if not a valid repo — we'll show that
  }

  const modeLabel = workspace.publish_mode === 'gh_pages_branch' ? 'gh-pages branch'
    : workspace.publish_mode === 'docs_folder' ? '/docs folder on main'
    : 'Not configured';

  // Build the detail page
  el.innerHTML = `
    <div class="w3-panel">
      <button class="w3-back" id="back-btn">← Back to Workspaces</button>

      <div style="display:flex;justify-content:space-between;align-items:flex-start;margin-bottom:16px;">
        <div>
          <h2 style="margin:0 0 4px;">${workspace.owner}/${workspace.repo}</h2>
          <span class="w3-ws-path">${workspace.local_path}</span>
        </div>
        ${workspace.site_url ? `<a href="#" class="w3-btn w3-btn-sm" id="open-site-btn">Open Site ↗</a>` : ''}
      </div>

      <!-- Info bar -->
      <div style="display:flex;gap:8px;margin-bottom:20px;flex-wrap:wrap;">
        <span class="w3-ws-branch">${workspace.branch}</span>
        <span class="w3-ws-mode">${modeLabel}</span>
        <span class="w3-ws-provider">${workspace.provider}</span>
      </div>

      <!-- Git Status Section -->
      <div id="git-section" style="margin-bottom:24px;">
        ${renderGitStatusSection(status, workspaceId)}
      </div>

      <!-- Publish Section -->
      <div id="publish-section">
        ${renderPublishSection(workspace)}
      </div>
    </div>
  `;

  // Bind events
  el.querySelector('#back-btn')?.addEventListener('click', () => navigateTo({ name: 'workspaces' }));

  el.querySelector('#open-site-btn')?.addEventListener('click', (e) => {
    e.preventDefault();
    if (workspace!.site_url) openSiteWindow(workspace!.site_url);
  });

  bindGitActions(el, workspaceId, workspace);
  bindPublishActions(el, workspace);
}

function renderGitStatusSection(status: GitStatus | null, workspaceId: string): string {
  if (!status) {
    return `
      <h3>Git</h3>
      <p class="w3-muted">Could not read git status for this workspace.</p>
      <button class="w3-btn w3-btn-sm" data-action="refresh-status" data-ws="${workspaceId}">Retry</button>
    `;
  }

  const { branch, dirty_files, ahead, behind, last_commit } = status;

  const filesHtml = dirty_files.length > 0
    ? `<div class="w3-file-list">${dirty_files.map((f) => `
        <div class="w3-file-item">
          <span class="w3-file-status w3-status-${f.status}">${f.status}</span>
          <span class="w3-file-path">${f.path}</span>
        </div>`).join('')}
      </div>`
    : '<p class="w3-muted" style="font-size:13px;">Working tree clean — no changes to commit.</p>';

  const lastCommitHtml = last_commit
    ? `<div class="w3-last-commit">
        Last commit: <code>${last_commit.sha.substring(0, 8)}</code> — ${last_commit.message}
        <br><small>${last_commit.author}, ${last_commit.date}</small>
      </div>`
    : '<p class="w3-muted" style="font-size:13px;">No commits yet.</p>';

  return `
    <h3>Git Status</h3>
    <div class="w3-status-bar">
      <span class="w3-ws-branch">${branch}</span>
      ${ahead > 0 ? `<span class="w3-ahead">↑ ${ahead} ahead</span>` : ''}
      ${behind > 0 ? `<span class="w3-behind">↓ ${behind} behind</span>` : ''}
      <span class="w3-file-count">${dirty_files.length} changed file${dirty_files.length !== 1 ? 's' : ''}</span>
    </div>
    ${filesHtml}
    ${lastCommitHtml}

    <!-- Commit form -->
    <div class="w3-commit-form" style="${dirty_files.length === 0 ? 'display:none;' : ''}">
      <input type="text" id="commit-msg" placeholder="Commit message" class="w3-input" />
      <div style="display:flex;gap:8px;margin-top:4px;">
        <button class="w3-btn" data-action="commit">Commit</button>
        <button class="w3-btn w3-btn-primary" data-action="commit-push">Commit &amp; Push</button>
      </div>
    </div>

    <!-- Push if ahead -->
    <div style="display:flex;gap:8px;margin-top:12px;">
      ${ahead > 0 ? `<button class="w3-btn" data-action="push">Push (${ahead} commit${ahead !== 1 ? 's' : ''})</button>` : ''}
      <button class="w3-btn w3-btn-sm" data-action="refresh-status">↻ Refresh</button>
    </div>

    <div class="w3-dialog-status" id="git-status-msg" style="display:none;"></div>
  `;
}

function bindGitActions(el: HTMLElement, workspaceId: string, workspace: Workspace): void {
  const gitSection = el.querySelector('#git-section')!;

  const getStatusEl = () => gitSection.querySelector('#git-status-msg') as HTMLElement | null;
  const getMsg = () => (gitSection.querySelector('#commit-msg') as HTMLInputElement | null)?.value.trim() ?? '';

  gitSection.querySelector('[data-action="refresh-status"]')?.addEventListener('click', async () => {
    try {
      const status = await api.getGitStatus(workspaceId);
      gitSection.innerHTML = renderGitStatusSection(status, workspaceId);
      bindGitActions(el, workspaceId, workspace);
    } catch (err) {
      const s = getStatusEl();
      if (s) showStatus(s, `Failed: ${err}`, true);
    }
  });

  gitSection.querySelector('[data-action="commit"]')?.addEventListener('click', async () => {
    const msg = getMsg();
    const s = getStatusEl();
    if (!msg) { if (s) showStatus(s, 'Please enter a commit message.', true); return; }
    if (s) showStatus(s, 'Committing...');
    try {
      const info = await api.commitChanges(workspaceId, msg);
      if (s) showStatus(s, `Committed: ${info.sha.substring(0, 8)} — ${info.message}`);
      // Refresh status after a moment
      setTimeout(async () => {
        const status = await api.getGitStatus(workspaceId);
        gitSection.innerHTML = renderGitStatusSection(status, workspaceId);
        bindGitActions(el, workspaceId, workspace);
      }, 1000);
    } catch (err) {
      if (s) showStatus(s, `Commit failed: ${err}`, true);
    }
  });

  gitSection.querySelector('[data-action="commit-push"]')?.addEventListener('click', async () => {
    const msg = getMsg();
    const s = getStatusEl();
    if (!msg) { if (s) showStatus(s, 'Please enter a commit message.', true); return; }
    if (s) showStatus(s, 'Committing and pushing...');
    try {
      const result = await api.commitAndPush(workspaceId, msg);
      if (result.push.success) {
        if (s) showStatus(s, `Pushed: ${result.commit.sha.substring(0, 8)} → ${result.push.remote}/${result.push.branch}`);
      } else {
        if (s) showStatus(s, `Committed but push failed: ${result.push.message}`, true);
      }
      setTimeout(async () => {
        const status = await api.getGitStatus(workspaceId);
        gitSection.innerHTML = renderGitStatusSection(status, workspaceId);
        bindGitActions(el, workspaceId, workspace);
      }, 1000);
    } catch (err) {
      if (s) showStatus(s, `Error: ${err}`, true);
    }
  });

  gitSection.querySelector('[data-action="push"]')?.addEventListener('click', async () => {
    const s = getStatusEl();
    if (s) showStatus(s, 'Pushing...');
    try {
      const result = await api.pushChanges(workspaceId);
      if (result.success) {
        if (s) showStatus(s, `Pushed to ${result.remote}/${result.branch}`);
      } else {
        if (s) showStatus(s, `Push failed: ${result.message}`, true);
      }
      setTimeout(async () => {
        const status = await api.getGitStatus(workspaceId);
        gitSection.innerHTML = renderGitStatusSection(status, workspaceId);
        bindGitActions(el, workspaceId, workspace);
      }, 1000);
    } catch (err) {
      if (s) showStatus(s, `Push failed: ${err}`, true);
    }
  });
}

// ─── Publish Section ───────────────────────────────────────────────────────

function renderPublishSection(workspace: Workspace): string {
  const modeLabel = workspace.publish_mode === 'gh_pages_branch' ? 'gh-pages branch'
    : workspace.publish_mode === 'docs_folder' ? '/docs folder on main'
    : 'Not configured';

  return `
    <h3>Publish</h3>
    <div class="w3-publish-info">
      <div class="w3-publish-row">
        <span class="w3-publish-label">Repository</span>
        <span>${workspace.owner}/${workspace.repo}</span>
      </div>
      <div class="w3-publish-row">
        <span class="w3-publish-label">Publish Mode</span>
        <span class="w3-ws-mode">${modeLabel}</span>
      </div>
      ${workspace.site_url ? `
        <div class="w3-publish-row">
          <span class="w3-publish-label">Site URL</span>
          <a href="#" data-site-url="${workspace.site_url}">${workspace.site_url}</a>
        </div>
      ` : ''}
    </div>
    <div style="display:flex;gap:8px;">
      <button class="w3-btn w3-btn-primary" data-action="publish">Publish / Update Site</button>
      <button class="w3-btn" data-action="detect-mode">Detect Mode</button>
    </div>
    <div class="w3-dialog-status" id="publish-status-msg" style="display:none;"></div>
  `;
}

function bindPublishActions(el: HTMLElement, workspace: Workspace): void {
  const section = el.querySelector('#publish-section')!;
  const getStatusEl = () => section.querySelector('#publish-status-msg') as HTMLElement | null;

  section.querySelector('[data-action="publish"]')?.addEventListener('click', async () => {
    const s = getStatusEl();
    if (s) showStatus(s, 'Publishing site...');
    try {
      const result: PublishResult = await api.publishSite(workspace.id);
      if (result.success) {
        let msg = `✓ Published! Mode: ${result.publish_mode.replace(/_/g, ' ')}`;
        if (result.site_url) msg += ` — ${result.site_url}`;
        if (s) showStatus(s, msg);
      } else {
        if (s) showStatus(s, `Publish failed: ${result.message}`, true);
      }
    } catch (err) {
      if (s) showStatus(s, `Publish failed: ${err}`, true);
    }
  });

  section.querySelector('[data-action="detect-mode"]')?.addEventListener('click', async () => {
    const s = getStatusEl();
    if (s) showStatus(s, 'Detecting publish mode...');
    try {
      const mode = await api.detectPublishMode(workspace.id);
      const label = mode === 'gh_pages_branch' ? 'gh-pages branch'
        : mode === 'docs_folder' ? '/docs folder on main'
        : 'none';
      if (s) showStatus(s, `Detected: ${label}`);
    } catch (err) {
      if (s) showStatus(s, `Detection failed: ${err}`, true);
    }
  });

  section.querySelectorAll('[data-site-url]').forEach((link) => {
    link.addEventListener('click', (e) => {
      e.preventDefault();
      const url = (e.currentTarget as HTMLElement).dataset.siteUrl;
      if (url) openSiteWindow(url);
    });
  });
}

// ─── Helpers ───────────────────────────────────────────────────────────────

function showStatus(el: HTMLElement, msg: string, isError = false): void {
  el.style.display = 'block';
  el.textContent = msg;
  el.className = `w3-dialog-status${isError ? ' w3-error' : ''}`;
}

// ─── Start ─────────────────────────────────────────────────────────────────

init();
