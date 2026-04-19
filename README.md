# wiki3-app

Desktop (Mobile planned) App for running Wiki3.ai sites.

## Overview

Wiki3 for Mac is a macOS desktop app built with **Tauri 2**. On launch it shows a **dashboard** where the user can open any GitHub-hosted Wiki3 site by pasting a repo URL. Each site opens in its own window. Open windows are restored across app launches.

The app also supports **Open**, **Run**, and **Publish** flows using the existing JupyterLab/JupyterLite platform, with desktop permission gating and persistent local state.

## Architecture

The app consists of four modular layers:

### 1. Tauri App Shell (`src-tauri/`)

The Rust backend that provides:

- Dashboard main window with repo URL input for opening sites
- Site windows opened from GitHub repo URLs (resolved via GitHub Pages API)
- Window state persistence — open site windows are restored on next launch
- Persistent app data directory for execution policy state and settings
- Origin-based trust verification (wiki3.ai and *.github.io)
- Tauri commands exposed to the frontend for desktop integration

### 2. Desktop Host Layer (`src-tauri/src/`)

Rust modules implementing the desktop host capabilities:

- **`config.rs`** — App configuration, trusted origin allowlist, dev URL override
- **`permissions.rs`** — Execution permission model (allow once / allow always / deny) and execution policy
- **`host.rs`** — Desktop host state management with persistent policy storage
- **`commands.rs`** — Tauri commands: host detection, permission state, execution policy, new window management, app settings
- **`window_state.rs`** — Window state persistence (open site windows, app settings) across app launches

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

- **Dashboard**: Main window with repo URL input prepopulated with the default site (`wiki3-ai/wiki3-ai-site`)
- **Open from Repo URL**: Paste any GitHub repo URL → the app resolves its GitHub Pages URL (via API, with custom domain support) and opens the site in a new window
- **Window Restore**: Site windows open at quit are automatically restored on next launch (configurable via `restore_windows` setting)
- **New Window Handling**: Links with `target="_blank"` and `window.open()` on wiki3.ai pages are intercepted and opened in real app windows (WKWebView workaround)
- **Run**: Enables notebook/cell execution through JupyterLite kernels (Pyodide/WASM Python, JavaScript) with desktop permission gating
- **Persistence**: JupyterLite IndexedDB/localStorage state survives app quit and relaunch
- **Security**: Trusted origin allowlist restricts desktop capabilities to wiki3.ai and *.github.io
- **Permission Gating**: User must approve execution (allow once / allow always / deny) before Run is enabled
- **Create from Template**: Create a new repo from `wiki3-ai/wiki3-ai-template`, clone, and open
- **Fork**: Fork any repo, poll until ready, clone with upstream remote
- **Commit & Push**: Stage, commit with message, push to origin (authenticated via token)
- **Publish**: Auto-detect gh-pages or docs-folder mode, build/deploy, report site URL

## Publishing Workflow

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

### Committing and Pushing

1. Edit files locally in the workspace directory
2. Open the workspace's "Commit & Push" panel to see dirty files and status
3. Enter a commit message, click "Commit & Push"
4. Push uses token-authenticated HTTPS (token is injected per-push, never stored in `.git/config`)

### Publishing / Updating a Site

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
