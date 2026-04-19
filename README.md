# wiki3-app

Desktop (Mobile planned) App for running Wiki3.ai sites.

## Overview

Wiki3 for Mac is a macOS desktop app built with **Tauri 2** that opens the [wiki3.ai](https://wiki3.ai) JupyterLite site, preserves the user's local JupyterLite state across app launches, and supports **Open**, **Run**, and **Publish** flows using the existing JupyterLab/JupyterLite platform.

## Architecture

The app consists of four modular layers:

### 1. Tauri App Shell (`src-tauri/`)

The Rust backend that provides:

- macOS desktop app window loading the trusted wiki3.ai URL
- Persistent app data directory for execution policy state
- Origin-based trust verification
- Tauri commands exposed to the frontend for desktop integration

### 2. Desktop Host Layer (`src-tauri/src/`)

Rust modules implementing the desktop host capabilities:

- **`config.rs`** — App configuration, trusted origin allowlist, dev URL override
- **`permissions.rs`** — Execution permission model (allow once / allow always / deny) and execution policy
- **`host.rs`** — Desktop host state management with persistent policy storage
- **`commands.rs`** — Tauri commands: host detection, permission state, execution policy

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

#### Publishing UI (`src/publishing/`)

- **`types.ts`** — Frontend types mirroring the Rust workspace/git models
- **`api.ts`** — Typed wrappers for all publishing Tauri commands
- **`ui/auth-panel.ts`** — GitHub token setup and authentication status
- **`ui/workspace-panel.ts`** — Workspace list, actions, and navigation
- **`ui/new-site-dialog.ts`** — "New Site from Template" dialog
- **`ui/fork-dialog.ts`** — "Fork Existing Site" dialog
- **`ui/commit-push-panel.ts`** — Git status, commit, push interface
- **`ui/publish-panel.ts`** — Site publish mode detection and publish trigger

## Features

- **Open**: Loads wiki3.ai in the desktop window, detects host presence, restores local state
- **Run**: Enables notebook/cell execution through JupyterLite kernels (Pyodide/WASM Python, JavaScript) with desktop permission gating
- **Persistence**: JupyterLite IndexedDB/localStorage state survives app quit and relaunch
- **Security**: Trusted origin allowlist restricts desktop capabilities to wiki3.ai only
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

## License

See [LICENSE](LICENSE) for details.
