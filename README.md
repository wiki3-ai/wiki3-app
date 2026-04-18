# wiki3-app

Desktop (Mobile planned) App for running Wiki3.ai sites.

## Overview

Wiki3 for Mac is a macOS desktop app built with **Tauri 2** that opens the [wiki3.ai](https://wiki3.ai) JupyterLite site, preserves the user's local JupyterLite state across app launches, and supports **Open** and **Run** flows using the existing JupyterLab/JupyterLite platform.

## Architecture

The app consists of three modular layers:

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

### 3. JupyterLab/JupyterLite Frontend Extension (`src/lib/`)

TypeScript modules for desktop integration:

- **`bridge.ts`** — Communication bridge between the JupyterLite frontend and Tauri host
- **`extension.ts`** — Desktop extension lifecycle: host detection, permission management, execution gating
- **`permission-dialog.ts`** — UI for requesting user permission before enabling execution
- **`types.ts`** — TypeScript type definitions for the integration layer

## Features

- **Open**: Loads wiki3.ai in the desktop window, detects host presence, restores local state
- **Run**: Enables notebook/cell execution through JupyterLite kernels (Pyodide/WASM Python, JavaScript) with desktop permission gating
- **Persistence**: JupyterLite IndexedDB/localStorage state survives app quit and relaunch
- **Security**: Trusted origin allowlist restricts desktop capabilities to wiki3.ai only
- **Permission Gating**: User must approve execution (allow once / allow always / deny) before Run is enabled

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
# Rust unit tests
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

## License

See [LICENSE](LICENSE) for details.
