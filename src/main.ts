/**
 * Wiki3 Desktop App — Dashboard UI
 *
 * Renders the list of wikis, provides actions to add / clone / open
 * local repos, and tracks per-wiki windows.
 */

import { listen } from '@tauri-apps/api/event';

import * as wikiApi from './wiki/api';
import type { TrackedWindowInfo, Wiki } from './wiki/types';

let wikis: Wiki[] = [];
let trackedWindows: TrackedWindowInfo[] = [];
// Wiki ids whose window-list section is currently expanded.
const expanded = new Set<string>();

const content = () => document.getElementById('main-content');

// ── Helpers ──────────────────────────────────────────────────────────────

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

function windowsForWiki(wikiId: string): TrackedWindowInfo[] {
  return trackedWindows.filter((w) => w.wiki_id === wikiId);
}

function deriveName(remoteUrl: string, localPath: string, siteUrl: string): string {
  const r = remoteUrl.trim();
  if (r) {
    const m = r.match(/[:/]([^/:]+)\/([^/]+?)(?:\.git)?\/?$/);
    if (m) return m[2];
  }
  const lp = localPath.trim();
  if (lp) {
    const parts = lp.split(/[\\/]+/).filter(Boolean);
    if (parts.length) return parts[parts.length - 1];
  }
  const su = siteUrl.trim();
  if (su) {
    try {
      const u = new URL(su);
      const segs = u.pathname.split('/').filter(Boolean);
      if (segs.length) return segs[segs.length - 1];
      return u.hostname;
    } catch {
      /* ignore */
    }
  }
  return 'my-garden';
}

// ── Rendering ────────────────────────────────────────────────────────────

function render(): void {
  const main = content();
  if (!main) return;

  if (wikis.length === 0) {
    main.innerHTML = `
      <div class="w3-empty">
        <h2>Welcome to Wiki3</h2>
        <p>No wikis on your dashboard yet. Add one to get started.</p>
        <div style="display:flex;gap:8px;justify-content:center;flex-wrap:wrap;">
          <button class="w3-btn w3-btn-primary" data-action="add-wiki">Add Wiki</button>
          <button class="w3-btn" data-action="clone-wiki">Clone from URL…</button>
          <button class="w3-btn" data-action="open-local">Open Local Repo…</button>
          <button class="w3-btn" data-action="restore-defaults">Restore defaults</button>
        </div>
      </div>`;
    return;
  }

  const cards = wikis.map(renderCard).join('');
  main.innerHTML = `
    <div class="w3-actions">
      <button class="w3-btn w3-btn-primary" data-action="add-wiki">Add Wiki…</button>
      <button class="w3-btn" data-action="clone-wiki">Clone from URL…</button>
      <button class="w3-btn" data-action="open-local">Open Local Repo…</button>
    </div>
    <div class="w3-workspace-list" id="w3-wiki-list">${cards}</div>
  `;
  wireDragAndDrop();
}

// ── Drag-and-drop reorder ────────────────────────────────────────────────

let dragSrcId: string | null = null;

function wireDragAndDrop(): void {
  const list = document.getElementById('w3-wiki-list');
  if (!list) return;
  const cards = Array.from(list.querySelectorAll<HTMLElement>('.w3-workspace-card'));

  cards.forEach((card) => {
    card.addEventListener('dragstart', (e) => {
      dragSrcId = card.getAttribute('data-wiki-id');
      card.classList.add('w3-dragging');
      if (e.dataTransfer) {
        e.dataTransfer.effectAllowed = 'move';
        // Needed for Firefox to initiate the drag.
        e.dataTransfer.setData('text/plain', dragSrcId ?? '');
      }
    });
    card.addEventListener('dragend', () => {
      card.classList.remove('w3-dragging');
      cards.forEach((c) => c.classList.remove('w3-drop-target'));
      dragSrcId = null;
    });
    card.addEventListener('dragover', (e) => {
      if (!dragSrcId) return;
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
      // Highlight the drop target.
      cards.forEach((c) => c.classList.remove('w3-drop-target'));
      card.classList.add('w3-drop-target');
    });
    card.addEventListener('dragleave', () => {
      card.classList.remove('w3-drop-target');
    });
    card.addEventListener('drop', (e) => {
      e.preventDefault();
      card.classList.remove('w3-drop-target');
      const srcId = dragSrcId;
      const dstId = card.getAttribute('data-wiki-id');
      dragSrcId = null;
      if (!srcId || !dstId || srcId === dstId) return;
      void reorderAfterDrop(srcId, dstId, e);
    });
  });
}

async function reorderAfterDrop(srcId: string, dstId: string, ev: DragEvent): Promise<void> {
  const srcIdx = wikis.findIndex((w) => w.id === srcId);
  const dstIdx = wikis.findIndex((w) => w.id === dstId);
  if (srcIdx < 0 || dstIdx < 0) return;

  // Decide whether to insert before or after the drop target based on
  // which half of the card the cursor is in.
  const dstEl = document
    .getElementById('w3-wiki-list')
    ?.querySelector<HTMLElement>(`.w3-workspace-card[data-wiki-id="${CSS.escape(dstId)}"]`);
  let before = true;
  if (dstEl) {
    const rect = dstEl.getBoundingClientRect();
    before = ev.clientY < rect.top + rect.height / 2;
  }

  const next = wikis.slice();
  const [moved] = next.splice(srcIdx, 1);
  // Recompute dst index after removal.
  let insertAt = next.findIndex((w) => w.id === dstId);
  if (insertAt < 0) insertAt = next.length;
  if (!before) insertAt += 1;
  next.splice(insertAt, 0, moved);

  // Optimistic render.
  wikis = next;
  render();

  try {
    await wikiApi.reorderWikis(next.map((w) => w.id));
  } catch (err) {
    console.error('Reorder failed, refreshing:', err);
    await refresh();
  }
}

function renderCard(w: Wiki): string {
  const hasRemote = !!w.remote;
  const hasLocal = !!w.local_path;
  const hasSite = !!w.site_url;

  const windows = windowsForWiki(w.id);
  const openCount = windows.filter((x) => !x.closed).length;
  const closedCount = windows.filter((x) => x.closed).length;
  const isExpanded = expanded.has(w.id);

  const siteBtn = hasSite || hasRemote
    ? `<button class="w3-btn w3-btn-primary w3-btn-sm" data-action="open-site" data-id="${escapeHtml(w.id)}">Open Site</button>`
    : '';

  const cloneBtn =
    hasRemote && !hasLocal
      ? `<button class="w3-btn w3-btn-sm" data-action="clone-to-local" data-id="${escapeHtml(w.id)}">Clone…</button>`
      : '';

  const revealBtn = hasLocal
    ? `<button class="w3-btn w3-btn-sm" data-action="reveal-local" data-id="${escapeHtml(w.id)}">Reveal Folder</button>`
    : '';

  const remoteBtn = hasRemote
    ? `<button class="w3-btn w3-btn-sm" data-action="open-remote" data-id="${escapeHtml(w.id)}">Open on GitHub</button>`
    : '';

  // Local-repo-only actions: commit / publish / build.
  const commitBtn = hasLocal
    ? `<button class="w3-btn w3-btn-sm" data-action="commit-wiki" data-id="${escapeHtml(w.id)}" title="Commit changes in the local repo">Commit…</button>`
    : '';
  const publishBtn = hasLocal && hasRemote
    ? `<button class="w3-btn w3-btn-sm" data-action="publish-wiki" data-id="${escapeHtml(w.id)}" title="Push and publish the site">Publish</button>`
    : '';
  const buildBtn = hasLocal
    ? `<button class="w3-btn w3-btn-sm" data-action="build-site" data-id="${escapeHtml(w.id)}" title="Run &#x60;jupyter lite build&#x60; in the local repo">Build Site</button>`
    : '';
  const pullBtn = hasLocal && hasRemote
    ? `<button class="w3-btn w3-btn-sm" data-action="pull-wiki" data-id="${escapeHtml(w.id)}" title="git pull origin">Pull</button>`
    : '';

  const pocCheckbox = hasLocal && hasRemote
    ? `<label class="w3-ws-poc" title="When checked, a successful Commit also pushes and publishes.">
         <input type="checkbox" data-action="toggle-publish-on-commit" data-id="${escapeHtml(w.id)}" ${w.publish_on_commit ? 'checked' : ''} />
         Publish on Commit
       </label>`
    : '';

  const closeAllBtn =
    openCount > 0
      ? `<button class="w3-btn w3-btn-sm" data-action="close-all" data-id="${escapeHtml(w.id)}">Close All (${openCount})</button>`
      : '';
  const reopenAllBtn =
    openCount === 0 && closedCount > 0
      ? `<button class="w3-btn w3-btn-sm" data-action="reopen-all" data-id="${escapeHtml(w.id)}">Reopen All (${closedCount})</button>`
      : '';

  const toggleLabel = isExpanded ? '▾' : '▸';
  const windowsToggle =
    windows.length > 0
      ? `<button class="w3-btn w3-btn-sm" data-action="toggle-windows" data-id="${escapeHtml(w.id)}" title="Show windows">${toggleLabel} Windows (${windows.length})</button>`
      : '';

  const windowsList = isExpanded && windows.length > 0
    ? `<div class="w3-windows-list" style="margin-top:8px;border-top:1px solid #eee;padding-top:8px;">${windows
        .map(renderWindowRow)
        .join('')}</div>`
    : '';

  const links: string[] = [];
  if (hasLocal) {
    links.push(
      `<div class="w3-ws-link-row"><span class="w3-ws-link-label">Local:</span> <a href="#" data-action="reveal-local" data-id="${escapeHtml(
        w.id,
      )}" class="w3-ws-path-link">${escapeHtml(w.local_path!)}</a></div>`,
    );
  }
  if (hasRemote) {
    links.push(
      `<div class="w3-ws-link-row"><span class="w3-ws-link-label">Remote:</span> <a href="#" data-action="open-remote" data-id="${escapeHtml(
        w.id,
      )}" class="w3-ws-url">${escapeHtml(w.remote!.url)}</a></div>`,
    );
  }
  if (hasSite) {
    links.push(
      `<div class="w3-ws-link-row"><span class="w3-ws-link-label">Site:</span> <a href="#" data-action="open-site" data-id="${escapeHtml(
        w.id,
      )}" class="w3-ws-url">${escapeHtml(w.site_url!)}</a></div>`,
    );
  }

  const originTag = originLabel(w.origin);

  return `
    <div class="w3-workspace-card" draggable="true" data-wiki-id="${escapeHtml(w.id)}">
      <div class="w3-ws-header">
        <span class="w3-drag-handle" title="Drag to reorder" aria-hidden="true">⋮⋮</span>
        <h3 style="margin:0;flex:1;">${escapeHtml(w.name)}</h3>
        <span class="w3-ws-provider">${originTag}</span>
      </div>
      ${w.description ? `<div class="w3-ws-meta"><span>${escapeHtml(w.description)}</span></div>` : ''}
      <div style="display:flex;flex-direction:column;gap:4px;margin:8px 0;">
        ${links.join('')}
      </div>
      <div class="w3-ws-actions">
        ${siteBtn}
        ${cloneBtn}
        ${commitBtn}
        ${publishBtn}
        ${buildBtn}
        ${pullBtn}
        ${remoteBtn}
        ${revealBtn}
        ${windowsToggle}
        ${closeAllBtn}
        ${reopenAllBtn}
        <button class="w3-btn w3-btn-sm w3-btn-danger" data-action="remove-wiki" data-id="${escapeHtml(w.id)}" title="Remove from dashboard">Remove</button>
      </div>
      ${pocCheckbox ? `<div class="w3-ws-poc-row">${pocCheckbox}</div>` : ''}
      ${windowsList}
    </div>`;
}

function originLabel(o: Wiki['origin']): string {
  if (typeof o === 'string') return o;
  if ('template' in o) return 'template';
  if ('fork' in o) return 'fork';
  return '';
}

function renderWindowRow(info: TrackedWindowInfo): string {
  const dot = info.closed
    ? '<span style="display:inline-block;width:8px;height:8px;border-radius:50%;background:#bbb;margin-right:8px;" title="Closed"></span>'
    : '<span style="display:inline-block;width:8px;height:8px;border-radius:50%;background:#4caf50;margin-right:8px;" title="Open"></span>';
  const action = info.closed ? 'Reopen' : 'Focus';
  return `
    <div class="w3-window-row" style="display:flex;align-items:center;justify-content:space-between;padding:4px 0;font-size:13px;">
      <div style="display:flex;align-items:center;min-width:0;">
        ${dot}
        <span style="overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">${escapeHtml(info.url)}</span>
      </div>
      <div style="display:flex;gap:4px;flex-shrink:0;">
        <button class="w3-btn w3-btn-sm" data-action="focus-window" data-label="${escapeHtml(info.label)}">${action}</button>
        <button class="w3-btn w3-btn-sm" data-action="forget-window" data-label="${escapeHtml(info.label)}" title="Forget this window">×</button>
      </div>
    </div>`;
}

// ── Data loading ─────────────────────────────────────────────────────────

async function refresh(): Promise<void> {
  try {
    [wikis, trackedWindows] = await Promise.all([
      wikiApi.listWikis(),
      wikiApi.listAllTrackedWindows(),
    ]);
  } catch (e) {
    console.error('Failed to load wikis:', e);
  }
  render();
}

// ── Dialogs ──────────────────────────────────────────────────────────────

function showDialog(innerHtml: string): HTMLElement {
  const overlay = document.createElement('div');
  overlay.className = 'w3-dialog-overlay';
  overlay.innerHTML = `<div class="w3-dialog">${innerHtml}</div>`;
  document.body.appendChild(overlay);
  overlay.addEventListener('click', (e) => {
    if (e.target === overlay) overlay.remove();
  });
  return overlay;
}

async function openAddWikiDialog(prefill?: Partial<{ remote: string; local: string; site: string }>): Promise<void> {
  const dlg = showDialog(`
    <h3>Add Wiki</h3>
    <p class="w3-muted" style="font-size:13px;margin-bottom:12px;">
      Provide at least one of the three. Any combination works — fields
      can be filled in later.
    </p>
    <form class="w3-form" id="add-wiki-form">
      <label>Name (optional)
        <input type="text" name="name" placeholder="Auto-derived if empty" />
      </label>
      <label>Local path
        <div style="display:flex;gap:6px;">
          <input type="text" name="local_path" value="${escapeHtml(prefill?.local ?? '')}" placeholder="/Users/me/Wiki3/my-garden" style="flex:1;" />
          <button type="button" class="w3-btn w3-btn-sm" data-act="pick">Browse…</button>
        </div>
      </label>
      <label>Remote URL
        <input type="text" name="remote_url" value="${escapeHtml(prefill?.remote ?? '')}" placeholder="https://github.com/owner/repo" />
      </label>
      <label>Site URL
        <input type="text" name="site_url" value="${escapeHtml(prefill?.site ?? '')}" placeholder="https://owner.github.io/repo" />
      </label>
      <label>Description (optional)
        <input type="text" name="description" />
      </label>
      <div class="w3-dialog-status" id="add-status" style="display:none;"></div>
      <div class="w3-dialog-actions">
        <button type="button" class="w3-btn" data-act="cancel">Cancel</button>
        <button type="submit" class="w3-btn w3-btn-primary">Add</button>
      </div>
    </form>`);

  const form = dlg.querySelector('#add-wiki-form') as HTMLFormElement;
  const status = dlg.querySelector('#add-status') as HTMLElement;
  form.querySelector('[data-act="cancel"]')!.addEventListener('click', () => dlg.remove());
  form.querySelector('[data-act="pick"]')!.addEventListener('click', async () => {
    try {
      const base = await wikiApi.getDefaultWikisDir();
      const picked = await wikiApi.pickFolder(base);
      if (picked) {
        (form.elements.namedItem('local_path') as HTMLInputElement).value = picked;
      }
    } catch (e) {
      console.error(e);
    }
  });
  form.addEventListener('submit', async (e) => {
    e.preventDefault();
    const fd = new FormData(form);
    try {
      await wikiApi.addWiki({
        name: (fd.get('name') as string) || null,
        local_path: (fd.get('local_path') as string) || null,
        remote_url: (fd.get('remote_url') as string) || null,
        site_url: (fd.get('site_url') as string) || null,
        description: (fd.get('description') as string) || null,
      });
      dlg.remove();
      await refresh();
    } catch (err) {
      status.style.display = 'block';
      status.classList.add('w3-error');
      status.textContent = String(err);
    }
  });
}

async function openCloneDialog(): Promise<void> {
  const url = window.prompt('Remote repo URL to clone:', 'https://github.com/wiki3-ai/wiki3-ai-template');
  if (!url || !url.trim()) return;

  const defaultBase = await wikiApi.getDefaultWikisDir().catch(() => '');
  const m = url.trim().match(/[:/]([^/:]+?)(?:\.git)?\/?$/);
  const defaultName = m ? m[1] : 'my-garden';
  const target = await wikiApi.pickCloneTarget(defaultBase, defaultName);
  if (!target) return;

  try {
    await wikiApi.cloneWiki(url.trim(), target);
    await refresh();
  } catch (err) {
    alert(`Clone failed: ${err}`);
  }
}

async function openLocalRepoDialog(): Promise<void> {
  const base = await wikiApi.getDefaultWikisDir().catch(() => undefined);
  const picked = await wikiApi.pickFolder(base);
  if (!picked) return;
  try {
    await wikiApi.openLocalRepoAsWiki(picked);
    await refresh();
  } catch (err) {
    alert(`Could not open: ${err}`);
  }
}

async function openCommitDialog(wikiId: string): Promise<void> {
  const w = wikis.find((x) => x.id === wikiId);
  if (!w) return;

  // Best-effort: show current git status so the user can sanity-check.
  let statusText = 'Loading status…';
  let hasChanges: boolean | null = null;
  try {
    const s = await wikiApi.wikiGitStatus(wikiId);
    const dirty = s.dirty_files.length + s.staged_files.length + s.untracked_files.length;
    hasChanges = dirty > 0;
    if (hasChanges) {
      statusText = `On ${escapeHtml(s.branch)}: ${s.staged_files.length} staged · ${s.dirty_files.length} modified · ${s.untracked_files.length} untracked`;
    } else {
      statusText = `On ${escapeHtml(s.branch)}: nothing to commit.`;
    }
  } catch (e) {
    statusText = `Could not read git status: ${escapeHtml(String(e))}`;
    hasChanges = null;
  }

  const poc = !!w.remote && w.publish_on_commit;
  const publishAvailable = !!w.remote;

  const dlg = showDialog(`
    <h3>Commit — ${escapeHtml(w.name)}</h3>
    <div class="w3-muted" style="font-size:13px;margin-bottom:8px;">${statusText}</div>
    <form class="w3-form" id="commit-form">
      <label>Commit message
        <input type="text" name="message" autofocus placeholder="Describe the change" required />
      </label>
      ${
        publishAvailable
          ? `<label class="w3-inline-check">
               <input type="checkbox" name="also_publish" ${poc ? 'checked' : ''} />
               Also publish (push and trigger site build)
             </label>`
          : `<div class="w3-muted" style="font-size:12px;">This wiki has no remote — commit only.</div>`
      }
      <div class="w3-dialog-status" id="commit-status" style="display:none;"></div>
      <div class="w3-dialog-actions">
        <button type="button" class="w3-btn" data-act="cancel">Cancel</button>
        <button type="submit" class="w3-btn w3-btn-primary"${hasChanges === false ? ' disabled' : ''}>Commit</button>
      </div>
    </form>`);

  const form = dlg.querySelector('#commit-form') as HTMLFormElement;
  const status = dlg.querySelector('#commit-status') as HTMLElement;
  form.querySelector('[data-act="cancel"]')!.addEventListener('click', () => dlg.remove());
  form.addEventListener('submit', async (e) => {
    e.preventDefault();
    const fd = new FormData(form);
    const message = (fd.get('message') as string) || '';
    const alsoPublish =
      publishAvailable && (fd.get('also_publish') === 'on' || fd.get('also_publish') === 'true');
    status.style.display = 'block';
    status.classList.remove('w3-error');
    status.textContent = alsoPublish ? 'Committing and publishing…' : 'Committing…';
    try {
      const result = await wikiApi.wikiCommitAndMaybePublish(wikiId, message, alsoPublish);
      dlg.remove();
      await refresh();
      if (result.published) {
        alert('Committed and pushed. The site will rebuild on the server — run Pull later to refresh the local copy.');
      }
    } catch (err) {
      status.classList.add('w3-error');
      status.textContent = String(err);
    }
  });
}

async function buildSite(wikiId: string): Promise<void> {
  const w = wikis.find((x) => x.id === wikiId);
  if (!w) return;
  const dlg = showDialog(`
    <h3>Building — ${escapeHtml(w.name)}</h3>
    <div class="w3-muted" style="font-size:13px;">Running <code>jupyter lite build</code> in <code>${escapeHtml(w.local_path ?? '')}</code>…</div>
    <div class="w3-dialog-status" id="build-status" style="display:block;margin-top:12px;">Working…</div>
    <div class="w3-dialog-actions">
      <button type="button" class="w3-btn" data-act="close" disabled>Close</button>
    </div>`);
  const status = dlg.querySelector('#build-status') as HTMLElement;
  const closeBtn = dlg.querySelector('[data-act="close"]') as HTMLButtonElement;
  try {
    const result = await wikiApi.wikiBuildSite(wikiId);
    status.textContent = `Built successfully → ${result.output_dir}`;
  } catch (err) {
    status.classList.add('w3-error');
    status.textContent = String(err);
  }
  closeBtn.disabled = false;
  closeBtn.addEventListener('click', () => dlg.remove());
}

async function openUrlDialog(): Promise<void> {
  const url = window.prompt('Wiki site or repo URL:');
  if (!url || !url.trim()) return;
  const trimmed = url.trim();
  // If it looks like a repo URL, register it as a wiki and open the site.
  const ghMatch = trimmed.match(/github\.com[:/]([^/]+)\/([^/]+?)(?:\.git)?\/?$/);
  if (ghMatch) {
    const wiki = await wikiApi.addWiki({
      remote_url: trimmed,
      site_url: null,
    });
    await refresh();
    try {
      await wikiApi.openWikiSite(wiki.id);
    } catch (err) {
      alert(`Failed to open: ${err}`);
    }
    return;
  }
  // Otherwise treat as a direct site URL.
  try {
    const wiki = await wikiApi.addWiki({ site_url: trimmed });
    await refresh();
    await wikiApi.openWikiSite(wiki.id);
  } catch (err) {
    alert(`Could not add/open: ${err}`);
  }
}

// ── Event handling ───────────────────────────────────────────────────────

async function handleAction(target: HTMLElement, ev: Event): Promise<void> {
  const action = target.getAttribute('data-action');
  if (!action) return;
  const id = target.getAttribute('data-id') || '';
  const label = target.getAttribute('data-label') || '';

  // Prevent default <a href="#"> navigation, but leave form controls alone.
  if (target.tagName !== 'INPUT' && target.tagName !== 'SELECT' && target.tagName !== 'TEXTAREA') {
    ev.preventDefault();
  }

  try {
    switch (action) {
      case 'add-wiki':
        await openAddWikiDialog();
        break;
      case 'clone-wiki':
        await openCloneDialog();
        break;
      case 'open-local':
        await openLocalRepoDialog();
        break;
      case 'open-url':
        await openUrlDialog();
        break;
      case 'restore-defaults':
        await wikiApi.restoreDefaultWikis();
        await refresh();
        break;
      case 'open-site':
        await wikiApi.openWikiSite(id);
        await refresh();
        break;
      case 'open-remote':
        await wikiApi.openWikiRemote(id);
        break;
      case 'reveal-local':
        await wikiApi.revealWikiLocal(id);
        break;
      case 'clone-to-local': {
        const w = wikis.find((x) => x.id === id);
        if (!w || !w.remote) return;
        const base = await wikiApi.getDefaultWikisDir().catch(() => '');
        const name = w.remote.repo || deriveName(w.remote.url, '', w.site_url ?? '');
        const target = await wikiApi.pickCloneTarget(base, name);
        if (!target) return;
        await wikiApi.cloneWiki(w.remote.url, target);
        // If the old entry was just a remote pointer, fill in its local_path.
        if (!w.local_path) {
          try {
            await wikiApi.updateWiki(id, { local_path: target });
          } catch {
            /* a new entry was already added by clone_wiki; ignore */
          }
        }
        await refresh();
        break;
      }
      case 'toggle-windows':
        if (expanded.has(id)) expanded.delete(id);
        else expanded.add(id);
        render();
        break;
      case 'close-all':
        await wikiApi.closeWikiWindows(id);
        await refresh();
        break;
      case 'reopen-all':
        await wikiApi.reopenWikiWindows(id);
        await refresh();
        break;
      case 'focus-window':
        await wikiApi.focusWindow(label);
        await refresh();
        break;
      case 'forget-window':
        await wikiApi.forgetTrackedWindow(label);
        await refresh();
        break;
      case 'remove-wiki': {
        if (!window.confirm('Remove this wiki from the dashboard? Local files are not deleted.'))
          return;
        await wikiApi.removeWiki(id);
        await refresh();
        break;
      }
      case 'commit-wiki':
        await openCommitDialog(id);
        break;
      case 'publish-wiki': {
        const go = window.confirm(
          'Publish this wiki?\n\nThis will push to origin and trigger the remote site build. Run Pull later to refresh your local copy once the build finishes.',
        );
        if (!go) return;
        try {
          await wikiApi.wikiPublish(id);
          await refresh();
          alert('Pushed. The site build has been triggered — run Pull later to refresh the local copy.');
        } catch (err) {
          alert(`Publish failed: ${err}`);
        }
        break;
      }
      case 'pull-wiki':
        try {
          const msg = await wikiApi.wikiPull(id);
          alert(`Pulled:\n${msg}`);
          await refresh();
        } catch (err) {
          alert(`Pull failed: ${err}`);
        }
        break;
      case 'build-site':
        await buildSite(id);
        break;
      case 'toggle-publish-on-commit': {
        const checked = (target as HTMLInputElement).checked;
        try {
          await wikiApi.setWikiPublishOnCommit(id, checked);
          const w = wikis.find((x) => x.id === id);
          if (w) w.publish_on_commit = checked;
        } catch (err) {
          // Revert the checkbox on error.
          (target as HTMLInputElement).checked = !checked;
          alert(String(err));
        }
        break;
      }
    }
  } catch (err) {
    console.error(`Action "${action}" failed:`, err);
    alert(String(err));
  }
}

// ── Menu event wiring ───────────────────────────────────────────────────

async function handleMenuAction(id: string): Promise<void> {
  switch (id) {
    case 'wiki3.new_from_template':
    case 'wiki3.clone_wiki':
    case 'wiki3.fork_wiki':
      // Fork/create-from-template requires GitHub auth; for now these
      // menu items share the clone flow which is always available.
      await openCloneDialog();
      break;
    case 'wiki3.open_local':
      await openLocalRepoDialog();
      break;
    case 'wiki3.open_url':
      await openUrlDialog();
      break;
    case 'wiki3.show_dashboard':
      // Backend already surfaces the dashboard; nothing to do here.
      break;
  }
}

// ── Init ─────────────────────────────────────────────────────────────────

async function init(): Promise<void> {
  console.log('[wiki3-app] Dashboard loading…');

  // Delegated click handling on the page.
  document.addEventListener('click', (e) => {
    const target = (e.target as HTMLElement | null)?.closest<HTMLElement>('[data-action]');
    if (target) {
      void handleAction(target, e);
    }
  });

  // Menu-driven actions from the backend.
  try {
    await listen<string>('wiki3://menu', (event) => {
      void handleMenuAction(event.payload);
    });
  } catch (e) {
    // Event plugin unavailable in pure web preview — safe to ignore.
    console.warn('Menu event listener not attached:', e);
  }

  await refresh();

  // Periodic refresh of window state in case of external changes.
  window.setInterval(() => {
    void refresh();
  }, 4000);
}

void init();
