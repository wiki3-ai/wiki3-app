# wiki3-app

Desktop (Mobile planned) App for running Wiki3.ai sites.

## Overview

Wiki3 for Mac is a macOS desktop app built with **Tauri 2**. On launch it shows a **dashboard** of your *wikis* — each wiki is a loose collection of up to three independent properties:

- a **local** git working copy,
- a **remote** git repository (e.g. GitHub), and
- a **site URL** (e.g. a GitHub Pages site).

Any one of the three is enough to create a dashboard entry; the others can be added later. Each wiki card exposes three link rows — Local (reveals in the OS file manager), Remote (opens in the system browser), and Site (opens in a new in-app window). Open/closed windows are tracked per wiki so you can close them all at once and reopen them in place.

On first launch, two wikis are seeded: `wiki3-ai/wiki3-ai-site` and `wiki3-ai/wiki3-ai-template`. Removing a seeded wiki does not bring it back — a "Restore defaults" action is provided for that. Entries from the older `workspaces.json` file (if present) are migrated once into `wikis.json`.

The app also supports **Open**, **Run**, and **Publish** flows using the existing JupyterLab/JupyterLite platform, with desktop permission gating and persistent local state.

## Architecture

The app consists of four modular layers:

### 1. Tauri App Shell (`src-tauri/`)

The Rust backend that provides:

- Dashboard main window with a list of wikis and per-wiki actions
- Site windows opened from a wiki's site URL (tagged with the owning wiki id)
- Per-wiki window tracking — close all / reopen all with geometry preserved
- Native app menu (File / View / Window / Help) with actions mirroring the dashboard
- Dashboard show/hide toggle (`⌘0`) and remembered dashboard geometry
- Window state persistence — open site windows are restored on next launch
- Persistent app data directory for wikis, execution policy state, and settings
- Origin-based trust verification (wiki3.ai, *.github.io, plus user-registered wiki site URLs)
- Tauri commands exposed to the frontend for desktop integration

### 2. Desktop Host Layer (`src-tauri/src/`)

Rust modules implementing the desktop host capabilities:

- **`config.rs`** — App configuration, trusted origin allowlist, dev URL override
- **`permissions.rs`** — Execution permission model (allow once / allow always / deny) and execution policy
- **`host.rs`** — Desktop host state management with persistent policy storage
- **`commands.rs`** — Tauri commands: host detection, permission state, execution policy, new window management, per-wiki window ops, dashboard toggle, external URL open, app settings
- **`window_state.rs`** — Window state persistence with per-window `wiki_id` / `closed` flags and separate dashboard geometry
- **`menu.rs`** — Native application menu construction and event routing
- **`wiki/`** — `Wiki` data model, `WikiManager` (CRUD / seeding / migration) and Tauri commands (`list_wikis`, `add_wiki`, `clone_wiki`, `open_wiki_site`, …)

### 3. Publishing & Workspace Layer (`src-tauri/src/`)

Rust modules implementing the Git-based publishing workflow:

- **`workspace/types.rs`** — Provider-neutral workspace model (provider type, publish mode, remotes, lineage)
- **`workspace/manager.rs`** — Workspace CRUD with local JSON persistence
- **`git/ops.rs`** — Git operations via CLI (clone, status, commit, push, remotes)
- **`providers/traits.rs`** — Abstract `RepoProvider` and `PublishProvider` traits
- **`providers/github/auth.rs`** — GitHub token storage (Keychain + file fallback), API client
- **`providers/github/repo.rs`** — `GitHubRepoProvider`: create from template, fork, repo info
- **`providers/github/publish.rs`** — `GitHubPagesPublishProvider`: detect mode, publish (gh-pages / docs folder)
- **`publishing_commands.rs`** — 15 Tauri commands for the full publishing workflow

### 4. Frontend Layers (`src/`)

TypeScript modules for desktop integration and publishing UI:

#### Desktop Integration (`src/lib/`)

- **`bridge.ts`** — Communication bridge between the JupyterLite frontend and Tauri host
- **`extension.ts`** — Desktop extension lifecycle: host detection, permission management, execution gating
- **`permission-dialog.ts`** — UI for requesting user permission before enabling execution
- **`types.ts`** — TypeScript type definitions for the integration layer

#### Dashboard (`src/main.ts`, `src/index.html`)

- **`main.ts`** — Dashboard entry point: loads saved settings, handles repo URL input and Open Site action
- **`index.html`** — Dashboard layout with repo URL bar (prepopulated with default repo URL)

#### Publishing API (`src/publishing/`)

- **`types.ts`** — Frontend types mirroring the Rust workspace/git models
- **`api.ts`** — Typed wrappers for all Tauri commands (publishing, settings, open-from-repo)

## Features

- **Dashboard**: List of wiki cards, each with links (local / remote / site), action buttons, and window tracking. New wikis appear at the top and can be dragged to reorder.
- **Per-wiki Git & Publish**: Local repos expose Commit, Push, Pull, Publish, and Build Site buttons. The commit dialog has an "Also publish" option, and each wiki has a persistent **Publish on Commit** checkbox so one click can do commit → push → site-build.
- **Build Site**: Runs `jupyter lite build` in the wiki's local directory to generate the static `_output/`.
- **Add Wiki / Clone / Open Local**: File-dialog driven flows defaulting to `~/Wiki3`. Wikis are loose records — any combination of local path / remote / site URL is valid.
- **Seeded Defaults**: First launch seeds `wiki3-ai/wiki3-ai-site` and `wiki3-ai/wiki3-ai-template`. Removing a default does not re-seed it.
- **Window Tracking**: Site windows opened from a wiki card are tagged to that wiki, shown in an expandable list, and can be Close All / Reopen All together. Geometry is preserved across close/reopen.
- **Window Restore**: Site windows open at quit are automatically restored on next launch (configurable via `restore_windows` setting).
- **New Window Handling**: Links with `target="_blank"` and `window.open()` on wiki3.ai pages are intercepted and opened in real app windows (WKWebView workaround).
- **Run**: Enables notebook/cell execution through JupyterLite kernels (Pyodide/WASM Python, JavaScript) with desktop permission gating.
- **Persistence**: JupyterLite IndexedDB/localStorage state survives app quit and relaunch.
- **Security**: Trusted origin allowlist restricts desktop capabilities to wiki3.ai and *.github.io.
- **Permission Gating**: User must approve execution (allow once / allow always / deny) before Run is enabled.
- **Create from Template / Fork**: Authenticated flows for creating or forking wikis on GitHub.
- **Native Menu**: File / View / Window / Help menus mirroring the dashboard buttons.

## Dashboard Flow

Each wiki card on the dashboard exposes a set of buttons driven by which of the three identifying properties (local path / remote / site URL) are set.

### Reordering

- **New wikis appear at the top** of the list (insert order).
- Drag any card by its header to reorder. The new order is persisted immediately via the `reorder_wikis` command; cards not mentioned in a partial reorder are preserved at the end.

### Commit (local-only)

1. Click **Commit…** on a card with a local path. The dialog shows the current branch and a summary of staged / modified / untracked files.
2. Enter a commit message. The dialog also offers an **Also publish** checkbox (pre-checked when the wiki's `publish_on_commit` flag is set).
3. Submit → backend runs `git add -A` + `git commit` in the wiki's `local_path`. If "Also publish" is set, it additionally pushes and enables GitHub Pages.

### Publish (local + remote)

- **Publish** pushes the current branch to `origin`, then best-effort enables GitHub Pages on the remote. The remote site build runs asynchronously on GitHub's side.
- **Pull** runs `git pull origin <branch>` to refresh the local copy once the remote build has finished.

### Publish on Commit

- Each wiki card with both a local path and a remote has a **Publish on Commit** checkbox under the action row.
- When checked, the commit dialog pre-checks "Also publish" so a single click does commit → push → Pages-enable in one round-trip.
- The flag is persisted on the wiki record (`publish_on_commit: bool`) via `set_wiki_publish_on_commit`.

### Build Site

- **Build Site** runs `jupyter lite build` in the wiki's local directory. The resulting static site is written to `_output/` (JupyterLite convention) and can be committed/published as usual.
- Requires `jupyter` and `jupyterlite-core` to be installed and on PATH; the app surfaces build output in the dialog.

### Planned follow-ups (not in this release)

The following pieces of the "local wiki as a live editing surface" vision need more design work and are tracked for future PRs:

- **Local preview server** — serving a built `_output/` directory to a new in-app window over `http://127.0.0.1:<port>/` so JupyterLite's service worker can work. Design decisions needed: which embedded HTTP server to adopt, per-wiki port lifecycle, and how the trusted-origin allowlist is extended for loopback URLs.
- **WebStorage ↔ local-file sync** — syncing notebook/markdown edits made in JupyterLite's IndexedDB/localStorage back to the wiki's local repo so they can be committed. This needs a coordinated change in the `wiki3-ai-site` repo's contents manager to route reads/writes through the desktop host bridge (see `src/lib/bridge.ts`).

The backend foundation for these (per-wiki git operations tied to a `local_path`) is already in place in this PR.

## Publishing Workflow (advanced)

The above dashboard flow is the common path. The following auth-based flows are available for creating new repos from templates or forking.

### GitHub Authentication Setup

1. Create a GitHub Personal Access Token (PAT) at <https://github.com/settings/tokens>
2. The token needs `repo` scope (full control of private repositories)
3. In the app, enter your token in the Auth panel — it's stored in the OS keychain (macOS Keychain, Linux Secret Service) or a secured local file

### Creating a Site from Template

1. Authenticate with GitHub
2. Click "New Site from Template"
3. Enter your GitHub username/org, repo name, and visibility
4. The default template is `wiki3-ai/wiki3-ai-template`
5. The app creates the repo, clones it to `~/Wiki3Sites/<repo-name>`, and records it as a workspace

### Forking an Existing Site

1. Authenticate with GitHub
2. Click "Fork Existing Site"
3. Enter the source repo (e.g. `wiki3-ai/wiki3-ai-site`)
4. The app forks, waits for provisioning, clones with both `origin` and `upstream` remotes

### Publishing / Updating a Site (advanced)

1. Open the workspace's "Publish" panel
2. The app auto-detects the publish mode:
   - **gh-pages branch**: Used when the repo has `deploy.sh`, `_output/`, or a workflow mentioning gh-pages
   - **/docs folder**: Used when the repo has a `/docs` directory
3. Click "Publish / Update" to push the site content
4. The resulting GitHub Pages URL is displayed

## Provider Abstraction

The publishing system is designed around abstract traits, making it straightforward to add new providers:

```
RepoProvider         — create from template, fork, get repo info
PublishProvider       — detect publish mode, publish, get site URL
```

Currently implemented: **GitHub** (`GitHubRepoProvider`, `GitHubPagesPublishProvider`)

### Adding a New Provider (e.g. Cloudflare/R2)

1. Create `src-tauri/src/providers/cloudflare/` with modules implementing `RepoProvider` and/or `PublishProvider`
2. Add `Cloudflare` variant to `ProviderType` enum in `workspace/types.rs`
3. Register new Tauri commands or add dispatch logic to existing commands
4. The workspace model, git operations, and UI components are provider-neutral and can be reused

Potential future providers:
- **Codeberg** — similar to GitHub, different API endpoints
- **Bare Git remotes** — RepoProvider using raw git protocol
- **Cloudflare R2** — PublishProvider for static file upload to R2 buckets
- **Cloudflare Artifacts** — RepoProvider alternative

## Development

### Prerequisites

- [Rust](https://rustup.rs/) (1.70+)
- [Node.js](https://nodejs.org/) (18+)
- macOS system libraries (for Tauri): on macOS these are bundled; on Linux install `libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev`

### Setup

```bash
npm install
```

### Run

```bash
npm run tauri:dev
```

### Development with dev URL

```bash
# Override the loaded URL for local development
export WIKI3_DEV_URL=http://localhost:8888
npm run tauri:dev
```

### Build

```bash
npm run tauri:build
```

### Test

```bash
# Rust unit tests (29 tests covering workspace, git, auth, publish, providers)
cd src-tauri && cargo test

# TypeScript type checking
npm run typecheck
```

## Configuration

| Variable | Description | Default |
|---|---|---|
| `WIKI3_DEV_URL` | Override site URL for development | (none — uses production URL) |
| Production URL | Trusted wiki3.ai site | `https://wiki3.ai` |
| App data directory | Persistent state location | OS-specific app data dir |
| Workspaces directory | Default location for cloned sites | `~/Wiki3Sites/` |

### App Settings (persisted in `window_state.json`)

| Setting | Description | Default |
|---|---|---|
| `restore_windows` | Reopen site windows from previous session on launch | `true` |
| `default_repo_url` | Prepopulated repo URL in the dashboard input | `https://github.com/wiki3-ai/wiki3-ai-site` |

## License

See [LICENSE](LICENSE) for details.
