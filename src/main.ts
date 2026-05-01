/**
 * Wiki3 Desktop App — Dashboard UI
 *
 * Renders the list of wikis, provides actions to add / clone / open
 * local repos, and tracks per-wiki windows.
 */

import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';

import * as wikiApi from './wiki/api';
import { computeReorder } from './wiki/dashboard-logic';
import type { TrackedWindowInfo, Wiki } from './wiki/types';
import { loadAndSubmitDevcontainer } from './devcontainer-engine';
import {
  toolsStatus,
  toolsCacheInfo,
  toolsClearCache,
  detectAppleContainer,
} from './lib/managed-tools';

let wikis: Wiki[] = [];
let trackedWindows: TrackedWindowInfo[] = [];
// Wiki ids whose window-list section is currently expanded.
const expanded = new Set<string>();
// Per-wiki preview-container status. Null/missing = not running.
const containerStatuses = new Map<string, wikiApi.RunningSite | null>();
const containerCtlStatuses = new Map<string, wikiApi.ContainerControlStatus | null>();
// Per-wiki forwarded-port snapshot.
const containerPorts = new Map<string, wikiApi.PortRow[]>();
/// Wikis with an in-flight container lifecycle action (Start /
/// Restart / Rebuild). While set, the dashboard offers a Cancel
/// button that fires `wiki_container_ctl_cancel` to break stuck
/// hooks (typically `postCreateCommand`).
const containerCtlInFlight = new Set<string>();

const content = () => document.getElementById('main-content');

// ── Logs panel ───────────────────────────────────────────────────────────

interface LogLine {
  wiki_id: string | null;
  source: string;
  level: 'stdout' | 'stderr' | 'info' | 'error';
  line: string;
  ts: number;
}

const LOG_MAX_LINES = 5000;function logsPanel(): HTMLElement | null {
  return document.getElementById('w3-logs-panel');
}

function logsBody(): HTMLElement | null {
  return document.getElementById('w3-logs-body');
}

function setLogsVisible(visible: boolean): void {
  const panel = logsPanel();
  if (!panel) return;
  panel.classList.toggle('open', visible);
  // Give the content area bottom padding so cards aren't hidden.
  const main = content();
  if (main) main.style.paddingBottom = visible ? '248px' : '';
}

function logsVisible(): boolean {
  return logsPanel()?.classList.contains('open') ?? false;
}

function appendLog(evt: LogLine): void {
  const body = logsBody();
  if (!body) return;
  const wiki = evt.wiki_id ? (wikis.find((w) => w.id === evt.wiki_id)?.name ?? evt.wiki_id) : '-';
  const cls =
    evt.level === 'stderr' || evt.level === 'error'
      ? 'w3-log-err'
      : evt.level === 'info'
        ? 'w3-log-info'
        : 'w3-log-out';
  const time = new Date(evt.ts).toLocaleTimeString();
  const row = document.createElement('div');
  row.className = cls;
  row.textContent = `[${time}] ${wiki} · ${evt.source}: ${evt.line}`;
  body.appendChild(row);
  // Cap the number of rendered rows to avoid DOM growth.
  while (body.childElementCount > LOG_MAX_LINES) {
    body.removeChild(body.firstChild as Node);
  }
  const auto = (document.getElementById('w3-logs-autoscroll') as HTMLInputElement | null)?.checked;
  if (auto !== false) {
    body.scrollTop = body.scrollHeight;
  }
}

function initLogsPanel(): void {
  document.getElementById('w3-logs-close')?.addEventListener('click', () => setLogsVisible(false));
  document.getElementById('w3-logs-clear')?.addEventListener('click', () => {
    const b = logsBody();
    if (b) b.innerHTML = '';
  });
  void listen<LogLine>('wiki:log', (e) => {
    appendLog(e.payload);
    // First log line of a session: auto-reveal so the user can see it.
    if (!logsVisible()) setLogsVisible(true);
  });
  // devcontainer-core's LifecycleOrchestrator emits its own event
  // names. Bridge them into the same panel so users see container
  // build / start / postStartCommand output here too.
  interface DevcontainerLogEvent {
    workspaceId: string;
    stream: 'stdout' | 'stderr' | 'system';
    line: string;
    ts: number;
  }
  void listen<DevcontainerLogEvent>('devcontainer://log', (e) => {
    const p = e.payload;
    appendLog({
      wiki_id: p.workspaceId,
      source: 'container',
      level: p.stream === 'stderr' ? 'stderr' : p.stream === 'system' ? 'info' : 'stdout',
      line: p.line,
      ts: p.ts,
    });
    if (!logsVisible()) setLogsVisible(true);
  });
}

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
    // `dragenter` and `dragover` must both call preventDefault() to mark
    // the card as a valid drop target in Chromium/WebKit.
    card.addEventListener('dragenter', (e) => {
      if (!dragSrcId) return;
      e.preventDefault();
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

  const next = computeReorder(wikis, srcId, dstId, before);
  // No-op: unchanged order.
  if (next.length === wikis.length && next.every((w, i) => w.id === wikis[i].id)) {
    return;
  }

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

  // Local-repo-only actions: commit.
  const commitBtn = hasLocal
    ? `<button class="w3-btn w3-btn-sm" data-action="commit-wiki" data-id="${escapeHtml(w.id)}" title="Commit changes in the local repo">Commit…</button>`
    : '';
  const ctlStatus = containerCtlStatuses.get(w.id) ?? null;
  const ctlState = ctlStatus?.state ?? 'unknown';
  const ctlRunning = ctlState === 'running';
  const ctlExists = ctlRunning || ctlState === 'stopped' || ctlState === 'created';

  // Forwarded-port monitor — sits above the container controls so the
  // user can see at a glance which services are reachable.
  const ports = containerPorts.get(w.id) ?? [];
  const portsBlock = hasLocal && ports.length > 0
    ? `<div class="w3-ws-ports" style="margin:8px 0;border:1px solid #e0e0e0;border-radius:4px;padding:6px 8px;font-size:13px;">
         <div style="font-weight:600;margin-bottom:4px;color:#555;">Ports</div>
         ${ports
           .map((p) => {
             const dotColor = p.serving ? '#4caf50' : '#bbb';
             const dotTitle = p.serving ? 'Serving' : 'Not reachable';
             const labelTxt = p.label ?? `Port ${p.external}`;
             const portCol = p.external === p.internal
               ? `${p.external}`
               : `${p.external} → ${p.internal}`;
             const link = p.serving
               ? `<a href="#" draggable="false" data-open-url="${escapeHtml(p.url)}" data-link-target="${escapeHtml(p.key)}" class="w3-ws-url" title="Open ${escapeHtml(p.url)}">${escapeHtml(p.url)}</a>`
               : `<span style="color:#999;">${escapeHtml(p.url)}</span>`;
             return `<div class="w3-ws-port-row" style="display:flex;align-items:center;gap:8px;padding:2px 0;">
               <span style="display:inline-block;width:8px;height:8px;border-radius:50%;background:${dotColor};flex-shrink:0;" title="${dotTitle}"></span>
               <span style="flex:0 0 auto;font-weight:500;">${escapeHtml(labelTxt)}</span>
               <span style="flex:0 0 auto;color:#666;font-family:ui-monospace,SFMono-Regular,Menlo,monospace;">${portCol}</span>
               <span style="flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">${link}</span>
             </div>`;
           })
           .join('')}
       </div>`
    : '';

  const containerRow = hasLocal
    ? `<div class="w3-ws-actions w3-ws-container-row">
         <span class="w3-ws-container-label" title="Devcontainer state">Container: <strong>${escapeHtml(ctlState)}</strong></span>
         <button class="w3-btn w3-btn-sm" data-action="container-up" data-id="${escapeHtml(w.id)}" ${ctlRunning ? 'disabled' : ''} title="Start (devcontainer up)">Start</button>
         <button class="w3-btn w3-btn-sm" data-action="container-stop" data-id="${escapeHtml(w.id)}" ${ctlRunning ? '' : 'disabled'} title="Stop the container">Stop</button>
         <button class="w3-btn w3-btn-sm" data-action="container-restart" data-id="${escapeHtml(w.id)}" ${ctlExists ? '' : 'disabled'} title="Stop then start">Restart</button>
         <button class="w3-btn w3-btn-sm" data-action="container-rebuild" data-id="${escapeHtml(w.id)}" title="Rebuild the image and recreate the container">Rebuild</button>
         <button class="w3-btn w3-btn-sm w3-btn-danger" data-action="container-remove" data-id="${escapeHtml(w.id)}" ${ctlExists ? '' : 'disabled'} title="Remove the container">Remove</button>${
        containerCtlInFlight.has(w.id)
          ? `<button class="w3-btn w3-btn-sm w3-btn-danger" data-action="container-cancel" data-id="${escapeHtml(w.id)}" title="Cancel the in-flight lifecycle hook (e.g. a stuck postCreateCommand)">Cancel</button>`
          : ''
      }
       </div>`
    : '';
  const pullBtn = hasLocal && hasRemote
    ? `<button class="w3-btn w3-btn-sm" data-action="pull-wiki" data-id="${escapeHtml(w.id)}" title="git pull origin">Pull</button>`
    : '';

  // Inline remote URL link, displayed alongside the git buttons.
  const remoteLink = hasRemote
    ? `<a href="#" draggable="false" data-action="open-remote" data-id="${escapeHtml(
        w.id,
      )}" class="w3-ws-url" style="margin-left:4px;align-self:center;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;min-width:0;" title="${escapeHtml(w.remote!.url)}">${escapeHtml(w.remote!.url)}</a>`
    : '';

  const pocCheckbox = hasLocal && hasRemote
    ? `<label class="w3-ws-poc" title="When checked, a successful Commit also pushes.">
         <input type="checkbox" data-action="toggle-publish-on-commit" data-id="${escapeHtml(w.id)}" ${w.publish_on_commit ? 'checked' : ''} />
         Publish on Commit
       </label>`
    : '';
  const autostartCheckbox = hasLocal
    ? `<div class="w3-ws-poc-row">
         <label class="w3-ws-poc" title="When checked, the preview container starts automatically when the app launches.">
           <input type="checkbox" data-action="toggle-autostart-container" data-id="${escapeHtml(w.id)}" ${w.autostart_container ? 'checked' : ''} />
           Autostart Container
         </label>
       </div>`
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
      `<div class="w3-ws-link-row"><span class="w3-ws-link-label">Local:</span> <a href="#" draggable="false" data-action="reveal-local" data-id="${escapeHtml(
        w.id,
      )}" class="w3-ws-path-link">${escapeHtml(w.local_path!)}</a></div>`,
    );
  }
  if (hasSite) {
    links.push(
      `<div class="w3-ws-link-row"><span class="w3-ws-link-label">Site:</span> <a href="#" draggable="false" data-action="open-site" data-id="${escapeHtml(
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
      ${portsBlock}
      ${containerRow}
      ${autostartCheckbox}
      <div class="w3-ws-actions">
        ${cloneBtn}
        ${commitBtn}
        ${pullBtn}
        ${remoteBtn}
        ${remoteLink}
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
  // Refresh container statuses for wikis with a local path. Failures
  // are non-fatal — we just leave the previous status in place.
  await Promise.all(
    wikis
      .filter((w) => !!w.local_path)
      .map(async (w) => {
        try {
          const s = await wikiApi.wikiContainerStatus(w.id);
          containerStatuses.set(w.id, s);
        } catch {
          /* keep previous status */
        }
        try {
          const c = await wikiApi.wikiContainerCtlStatus(w.id);
          containerCtlStatuses.set(w.id, c);
        } catch {
          /* keep previous status */
        }
        try {
          const ports = await wikiApi.wikiContainerPorts(w.id);
          containerPorts.set(w.id, ports);
        } catch {
          /* keep previous ports */
        }
      }),
  );
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

/**
 * Full-screen modal shown while the backend tears down preview
 * containers on app quit. Blocks all interaction (no dismiss-on-click)
 * and stays up until the process exits.
 */
function showShutdownOverlay(): void {
  if (document.getElementById('w3-shutdown-overlay')) return;
  const overlay = document.createElement('div');
  overlay.id = 'w3-shutdown-overlay';
  overlay.style.cssText = [
    'position:fixed',
    'inset:0',
    'background:rgba(20,20,28,0.85)',
    'color:#fff',
    'z-index:99999',
    'display:flex',
    'flex-direction:column',
    'align-items:center',
    'justify-content:center',
    'gap:16px',
    'font-family:-apple-system,BlinkMacSystemFont,sans-serif',
    'cursor:wait',
  ].join(';');
  overlay.innerHTML = `
    <div style="width:48px;height:48px;border:4px solid rgba(255,255,255,0.2);border-top-color:#fff;border-radius:50%;animation:w3-spin 1s linear infinite;"></div>
    <div style="font-size:16px;font-weight:600;">Shutting down…</div>
    <div style="font-size:13px;opacity:0.85;max-width:360px;text-align:center;line-height:1.4;">
      Stopping preview containers and Apple Container service.
      This usually takes a few seconds.
    </div>
    <style>@keyframes w3-spin { to { transform: rotate(360deg); } }</style>
  `;
  // Capture-phase listeners on the overlay swallow any stray clicks.
  overlay.addEventListener('click', (e) => e.stopPropagation(), true);
  overlay.addEventListener('keydown', (e) => e.stopPropagation(), true);
  document.body.appendChild(overlay);
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

async function startContainer(wikiId: string): Promise<void> {
  const w = wikis.find((x) => x.id === wikiId);
  if (!w) return;
  const dlg = showDialog(`
    <h3>Starting Preview Container — ${escapeHtml(w.name)}</h3>
    <div class="w3-muted" style="font-size:13px;">
      Starting <code>jupyter lite serve</code> in Apple Container.
      The first run can take a while while the image is built and
      <code>jupyter lite build</code> completes. The Site button will
      appear on the card once the server is accepting connections.
    </div>
    <div class="w3-dialog-status" id="serve-status" style="display:block;margin-top:12px;">Working…</div>
    <div class="w3-dialog-actions">
      <button type="button" class="w3-btn" data-act="close" disabled>Close</button>
    </div>`);
  const status = dlg.querySelector('#serve-status') as HTMLElement;
  const closeBtn = dlg.querySelector('[data-act="close"]') as HTMLButtonElement;
  try {
    const site = await wikiApi.wikiStartContainer(wikiId);
    containerStatuses.set(wikiId, site);
    status.textContent = `Serving on ${site.url}`;
    await refresh();
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

// ── Tools dialog ─────────────────────────────────────────────────────────

/**
 * Open the bundled-tools dialog. Shows:
 *   - Deno: bundled with the app, read-only version + path
 *   - Apple Container: detection status + "Re-check" button
 *   - Disposable cache: size + "Clear cache" button
 *
 * The dialog deliberately has no "install" buttons: Deno is shipped
 * inside the .app (see `build.rs`), not installed by the user.
 */
async function openToolsDialog(): Promise<void> {
  const dlg = showDialog(`
    <h3>Bundled Tools</h3>
    <p class="w3-muted" style="font-size:13px;margin-bottom:12px;">
      Deno is bundled inside Wiki3 and is always available. Apple
      Container is a system-wide dependency installed once via its
      signed <code>.pkg</code>.
    </p>
    <div id="w3-tools-body" style="font-size:13px;">
      <div class="w3-muted">Loading…</div>
    </div>
    <div class="w3-dialog-status" id="w3-tools-status" style="display:none;"></div>
    <div class="w3-dialog-actions">
      <button type="button" class="w3-btn" data-tools-act="close">Close</button>
    </div>`);

  const body = dlg.querySelector('#w3-tools-body') as HTMLElement;
  const statusEl = dlg.querySelector('#w3-tools-status') as HTMLElement;
  const closeBtn = dlg.querySelector('[data-tools-act="close"]') as HTMLButtonElement;

  const showStatus = (text: string, isError = false) => {
    statusEl.style.display = 'block';
    statusEl.textContent = text;
    statusEl.classList.toggle('w3-error', isError);
  };
  const clearStatus = () => {
    statusEl.style.display = 'none';
    statusEl.textContent = '';
    statusEl.classList.remove('w3-error');
  };

  closeBtn.addEventListener('click', () => dlg.remove());

  const refreshBody = async () => {
    body.innerHTML = `<div class="w3-muted">Loading…</div>`;
    try {
      const [tools, ac, cache] = await Promise.all([
        toolsStatus(),
        detectAppleContainer(),
        toolsCacheInfo(),
      ]);
      body.innerHTML =
        renderToolsRows(tools) +
        renderAppleContainerRow(ac) +
        renderCacheRow(cache);
    } catch (err) {
      body.innerHTML = `<div class="w3-error">Failed to load tool status: ${escapeHtml(String(err))}</div>`;
    }
  };

  const setBusy = (busy: boolean) => {
    dlg
      .querySelectorAll<HTMLButtonElement>('button[data-tools-act], button[data-tools-row-act]')
      .forEach((b) => (b.disabled = busy));
  };

  body.addEventListener('click', async (e) => {
    // Clickable external links: <a target="_blank"> doesn't work in
    // the Tauri WebView, so we route URLs through the backend's
    // `open_url` command (which calls `/usr/bin/open`).
    const link = (e.target as HTMLElement | null)?.closest<HTMLElement>(
      '[data-open-url]',
    );
    if (link) {
      e.preventDefault();
      const url = link.getAttribute('data-open-url') || '';
      if (url) {
        try {
          await invoke('open_url', { url });
        } catch (err) {
          console.warn('open_url failed:', err);
        }
      }
      return;
    }

    const btn = (e.target as HTMLElement | null)?.closest<HTMLButtonElement>(
      '[data-tools-row-act]',
    );
    if (!btn) return;
    const act = btn.getAttribute('data-tools-row-act');
    if (!act) return;
    setBusy(true);
    clearStatus();
    try {
      if (act === 'recheck-container') {
        // fall through to refresh
      } else if (act === 'clear-cache') {
        if (
          !window.confirm(
            'Clear the Deno + npm caches? They will be repopulated on the next build.',
          )
        ) {
          setBusy(false);
          return;
        }
        await toolsClearCache();
        showStatus('Cache cleared.');
      }
      await refreshBody();
    } catch (err) {
      showStatus(String(err), true);
    } finally {
      setBusy(false);
    }
  });

  await refreshBody();
}

type ToolsStatusRow = Awaited<ReturnType<typeof toolsStatus>>[number];
type AppleContainerRow = Awaited<ReturnType<typeof detectAppleContainer>>;
type CacheRow = Awaited<ReturnType<typeof toolsCacheInfo>>;

function renderToolsRows(rows: ToolsStatusRow[]): string {
  if (rows.length === 0) {
    return `<div class="w3-muted">No bundled tools.</div>`;
  }
  const items = rows
    .map((r) => {
      const pathLine = r.path
        ? `<div style="font-size:11px;color:#666;font-family:monospace;word-break:break-all;">${escapeHtml(r.path)}</div>`
        : `<div class="w3-error" style="font-size:12px;">Bundled binary missing — this app build is broken.</div>`;
      return `
        <div class="w3-workspace-card" style="padding:12px;">
          <div class="w3-ws-header" style="margin-bottom:4px;">
            <strong>${escapeHtml(r.name)}</strong>
            <span class="w3-ws-provider">bundled v${escapeHtml(r.version)}</span>
          </div>
          ${pathLine}
        </div>`;
    })
    .join('');
  return `<div class="w3-workspace-list" style="margin-bottom:12px;">${items}</div>`;
}

function renderAppleContainerRow(ac: AppleContainerRow): string {
  const label = ac.installed
    ? `<span style="color:#2e7d32;">Installed</span>${
        ac.path ? ` — <code style="font-size:11px;">${escapeHtml(ac.path)}</code>` : ''
      }`
    : `<span style="color:#e65100;">Not installed</span> — needed for sandboxed builds. Install from <a href="https://github.com/apple/container" data-open-url="https://github.com/apple/container" style="cursor:pointer;color:#1976d2;text-decoration:underline;">apple/container</a>.`;
  return `
    <div class="w3-workspace-card" style="padding:12px;margin-bottom:12px;">
      <div class="w3-ws-header" style="margin-bottom:4px;">
        <strong>Apple Container</strong>
        <span class="w3-ws-provider">system</span>
      </div>
      <div style="font-size:12px;margin-bottom:8px;">${label}</div>
      <div class="w3-ws-actions">
        <button class="w3-btn w3-btn-sm" data-tools-row-act="recheck-container">Re-check</button>
      </div>
    </div>`;
}

function renderCacheRow(c: CacheRow): string {
  const sizeMb = c.size_bytes > 0 ? `${(c.size_bytes / 1_000_000).toFixed(1)} MB` : 'empty';
  return `
    <div class="w3-workspace-card" style="padding:12px;">
      <div class="w3-ws-header" style="margin-bottom:4px;">
        <strong>Build cache</strong>
        <span class="w3-ws-provider">${escapeHtml(sizeMb)}</span>
      </div>
      <div style="font-size:11px;color:#666;font-family:monospace;word-break:break-all;margin-bottom:8px;">
        ${escapeHtml(c.path)}
      </div>
      <div class="w3-ws-actions">
        <button class="w3-btn w3-btn-sm w3-btn-danger" data-tools-row-act="clear-cache" ${c.size_bytes === 0 ? 'disabled' : ''}>Clear cache</button>
      </div>
    </div>`;
}

// ── Event handling ───────────────────────────────────────────────────────

/**
 * Read `.devcontainer/devcontainer.json` for `wikiId`, parse it via
 * the embedded engine, and submit the result to Rust so the
 * orchestrator picks up the latest config (including any changes the
 * user just made on disk). Container-mutating actions call this
 * before invoking the corresponding `wiki_container_ctl_*` command.
 */
async function ensureDevcontainerSubmitted(wikiId: string): Promise<void> {
  const w = wikis.find((x) => x.id === wikiId);
  if (!w?.local_path) return;
  await loadAndSubmitDevcontainer(wikiId, w.local_path);
}

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
      case 'open-tools':
        await openToolsDialog();
        break;
      case 'toggle-logs':
        setLogsVisible(!logsVisible());
        break;
      case 'restore-defaults':
        await wikiApi.restoreDefaultWikis();
        await refresh();
        break;
      case 'open-site': {
        const w = wikis.find((x) => x.id === id);
        const url = w?.site_url ?? null;
        if (url) {
          await wikiApi.openExternalUrl(url);
        } else {
          await wikiApi.openWikiSite(id);
        }
        await refresh();
        break;
      }
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
          'Publish this wiki?\n\nThis pushes the current branch to origin. Site rebuilds happen via whatever CI you have configured on the remote (if any).',
        );
        if (!go) return;
        try {
          await wikiApi.wikiPublish(id);
          await refresh();
          alert('Pushed to origin.');
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
      case 'start-container':
        await startContainer(id);
        break;
      case 'stop-container':
        try {
          await wikiApi.wikiStopContainer(id);
          containerStatuses.set(id, null);
          await refresh();
        } catch (err) {
          alert(`Stop failed: ${err}`);
        }
        break;
      case 'open-local-site-external': {
        const status = containerStatuses.get(id);
        if (status?.url) {
          await wikiApi.openExternalUrl(status.url);
        }
        break;
      }
      case 'container-up':
        try {
          containerCtlInFlight.add(id);
          render();
          await ensureDevcontainerSubmitted(id);
          const s = await wikiApi.wikiContainerCtlUp(id);
          containerCtlStatuses.set(id, s);
        } catch (err) {
          alert(`Start failed: ${err}`);
        } finally {
          containerCtlInFlight.delete(id);
          render();
        }
        break;
      case 'container-stop':
        try {
          const s = await wikiApi.wikiContainerCtlStop(id);
          containerCtlStatuses.set(id, s);
          render();
        } catch (err) {
          alert(`Stop failed: ${err}`);
        }
        break;
      case 'container-restart':
        try {
          containerCtlInFlight.add(id);
          render();
          await ensureDevcontainerSubmitted(id);
          const s = await wikiApi.wikiContainerCtlRestart(id);
          containerCtlStatuses.set(id, s);
        } catch (err) {
          alert(`Restart failed: ${err}`);
        } finally {
          containerCtlInFlight.delete(id);
          render();
        }
        break;
      case 'container-rebuild':
        if (!window.confirm('Rebuild the image and recreate the container? Any in-container state is lost.')) return;
        try {
          containerCtlInFlight.add(id);
          render();
          await ensureDevcontainerSubmitted(id);
          const s = await wikiApi.wikiContainerCtlRebuild(id);
          containerCtlStatuses.set(id, s);
        } catch (err) {
          alert(`Rebuild failed: ${err}`);
        } finally {
          containerCtlInFlight.delete(id);
          render();
        }
        break;
      case 'container-cancel':
        try {
          const fired = await wikiApi.wikiContainerCtlCancel(id);
          if (!fired) {
            // Nothing was registered — the action probably finished
            // between render and click. Re-render so the button
            // disappears.
            render();
          }
        } catch (err) {
          alert(`Cancel failed: ${err}`);
        }
        break;
      case 'container-remove':
        if (!window.confirm('Remove this container? It will need to be rebuilt next time.')) return;
        try {
          const s = await wikiApi.wikiContainerCtlRemove(id);
          containerCtlStatuses.set(id, s);
          render();
        } catch (err) {
          alert(`Remove failed: ${err}`);
        }
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
      case 'toggle-autostart-container': {
        const checked = (target as HTMLInputElement).checked;
        try {
          await wikiApi.setWikiAutostartContainer(id, checked);
          const w = wikis.find((x) => x.id === id);
          if (w) w.autostart_container = checked;
        } catch (err) {
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

  initLogsPanel();

  // Delegated click handling on the page.
  document.addEventListener('click', (e) => {
    const openUrlEl = (e.target as HTMLElement | null)?.closest<HTMLElement>('[data-open-url]');
    if (openUrlEl) {
      e.preventDefault();
      const url = openUrlEl.getAttribute('data-open-url') || '';
      if (url) {
        void invoke('open_url', { url }).catch((err) => {
          console.error('open_url failed:', err);
        });
      }
      return;
    }
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

  // Shutdown overlay: the backend emits this after it intercepts the
  // exit and starts tearing down preview containers, which can take
  // several seconds. Block all UI so the user knows the app hasn't
  // hung. The window will close on its own once cleanup finishes.
  try {
    await listen<unknown>('wiki3://shutdown-begin', () => {
      showShutdownOverlay();
    });
  } catch (e) {
    console.warn('Shutdown event listener not attached:', e);
  }

  await refresh();

  // Periodic refresh of window state in case of external changes.
  window.setInterval(() => {
    void refresh();
  }, 4000);
}

void init();
