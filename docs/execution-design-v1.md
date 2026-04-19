# Execution Design & Implementation

How code runs in Wiki3 for Mac — today and in the future.

---

## Part 1 — Current Architecture

### How the Desktop App Runs Code

Wiki3 for Mac is a **Tauri 2** desktop app. It does **not** launch Python interpreters, spawn kernel processes, or run containers. Instead it opens a WebView window pointing at a remote JupyterLite site and acts as a permission-gated host.

```
┌──────────────────────────────────────────────────┐
│  Tauri App Process (Rust)                        │
│                                                  │
│  ┌──────────────┐  ┌─────────────────────────┐   │
│  │ DesktopHost  │  │ PublishingState         │   │
│  │  State       │  │  WorkspaceManager       │   │
│  │  • config    │  │  GitHubAuth / Keychain  │   │
│  │  • policy    │  │  git CLI operations     │   │
│  └──────┬───────┘  └────────────┬────────────┘   │
│         │  Tauri IPC (invoke)   │                │
│  ═══════╪═══════════════════════╪════════════    │
│         │                       │                │
│  ┌──────┴───────────────────────┴────────────┐   │
│  │           WKWebView (macOS)               │   │
│  │                                           │   │
│  │   wiki3.ai  ──  JupyterLite application   │   │
│  │   ┌───────────────────────────────────┐   │   │
│  │   │  JupyterLab UI  (TypeScript)      │   │   │
│  │   │  ┌─────────┐  ┌───────────────┐   │   │   │
│  │   │  │ Notebook │  │ Desktop Ext. │   │   │   │
│  │   │  │ Editor   │  │ (bridge.ts)  │   │   │   │
│  │   │  └────┬─────┘  └──────────────┘   │   │   │
│  │   │       │                           │   │   │
│  │   │  ┌────┴─────────────────────┐     │   │   │
│  │   │  │  JupyterLite Kernels     │     │   │   │
│  │   │  │  • Pyodide (WASM Python) │     │   │   │
│  │   │  │  • JavaScript            │     │   │   │
│  │   │  └──────────────────────────┘     │   │   │
│  │   └───────────────────────────────────┘   │   │
│  └───────────────────────────────────────────┘   │
└──────────────────────────────────────────────────┘
```

#### Boot Sequence

1. Tauri creates the dashboard window loading the local `index.html`.
2. User pastes a GitHub repo URL → the app resolves its GitHub Pages URL and opens a site window via `WebviewWindowBuilder`.
3. The site window loads `https://wiki3.ai` (or a `*.github.io` Pages URL), which serves JupyterLite.
4. On page load, Tauri injects a navigation handler for `target="_blank"` links (`lib.rs` `on_page_load`).
5. JupyterLite starts up inside the WebView, initializing its own extensions and kernels.

#### Desktop Integration Bridge

The TypeScript layer in `src/lib/` provides a bridge between the JupyterLite frontend and the Tauri host. It detects whether `window.__TAURI_INTERNALS__` is available, meaning the site is running inside the desktop app rather than a normal browser.

The bridge exposes three operations over Tauri IPC:

| Bridge method | Tauri command | Purpose |
|---|---|---|
| `detectHost()` | `detect_desktop_host` | Confirm desktop context + origin trust |
| `getExecutionState()` | `get_execution_state` | Query whether execution is allowed |
| `setPermission(choice)` | `set_execution_permission` | Record user's allow/deny decision |

The `Wiki3DesktopExtension` class orchestrates the lifecycle: detect host → check permission → gate execution. The `PermissionDialog` provides UI for the allow-once / allow-always / deny prompt.

#### Trust Model

Only two origins are treated as trusted for the desktop bridge:

- `https://wiki3.ai`
- `https://www.wiki3.ai`

(Defined in `config.rs` as `TRUSTED_ORIGINS`.)

Site windows can also be opened for `*.github.io` origins, but these are **not** trusted for the bridge commands — they can display content but cannot call `detect_desktop_host` etc.

#### What the Tauri Host Does NOT Do

- Does not launch Python, Node.js, or any interpreter process.
- Does not manage kernel lifecycle, restart, or interrupt.
- Does not provide filesystem access to kernel code.
- Does not run containers of any kind.
- Does not sandbox kernel code beyond the WebView boundary.

---

### JupyterLite: How It Works

JupyterLite is a full JupyterLab distribution that runs entirely in the browser. Wiki3 uses a remotely-hosted JupyterLite deployment (served from wiki3.ai or GitHub Pages).

#### Key Components

**JupyterLite Core** — A static build of JupyterLab compiled to run without a server. It uses Service Workers and IndexedDB to emulate the Jupyter server API inside the browser.

**Extensions** — Standard JupyterLab extensions, bundled at build time. The Wiki3 desktop integration extension (`src/lib/`) is one of these — it hooks into JupyterLab's lifecycle to gate execution based on the host's permission state.

**Notebooks** — `.ipynb` files stored in the browser's IndexedDB (the emulated "filesystem"). They persist across page reloads and app launches because IndexedDB state is scoped to the origin and survives WebView teardown.

**Kernels** — JupyterLite provides in-browser kernel implementations:

| Kernel | Runtime | How it works |
|---|---|---|
| **Pyodide** | WebAssembly | CPython compiled to WASM via Emscripten. Runs a full Python 3.x interpreter inside the browser. Packages are loaded from PyPI via micropip. I/O is emulated. |
| **JavaScript** | Browser JS engine | Executes code using the page's own JavaScript runtime (`eval` or Web Worker). Has direct access to browser APIs. |

Both kernels run **in the same origin and process** as the JupyterLite UI. There is no kernel process, no ZeroMQ, no separate address space. The "kernel" is a JavaScript class that receives execute requests and produces outputs, using `postMessage` to a Web Worker at most.

#### Isolation Properties

| Property | Pyodide (WASM) | JavaScript |
|---|---|---|
| Separate OS process | No | No |
| Separate thread | Web Worker (optional) | Web Worker (optional) |
| Memory isolation | WASM linear memory (sandboxed from JS heap) | None — shares page JS context |
| Can access DOM | Via JS interop bridge | Yes, directly |
| Can call `fetch()` | Yes (subject to CORS) | Yes |
| Can access IndexedDB | Yes | Yes |
| Can see `__TAURI_INTERNALS__` | Via JS interop | Yes, directly |
| Can exhaust CPU/memory | Yes | Yes |
| Can escape WebView sandbox | Only via browser/WASM engine bug | Only via browser engine bug |

**Key point:** WASM provides language-level sandboxing (no raw pointer access to the host process) but does not provide OS-level isolation. Both kernel types live inside the same WebView, have the same network access, and are bounded only by the browser/WebView sandbox.

---

## Part 2 — Adding More Kernel Execution Options

There are three tiers of kernel isolation, each with increasing capability and complexity.

### Tier 1: In-Browser Kernels (Current)

These are the existing JupyterLite kernels. No changes needed to the Tauri host.

#### JS/TS Kernel (exists today)

JupyterLite's JavaScript kernel already supports ES module syntax. To add TypeScript support:

1. Bundle a TypeScript transpiler (e.g., `typescript` compiler or `sucrase`) into the JupyterLite build.
2. Create a JupyterLite kernel extension that transpiles `.ts` cell contents to JS before execution.
3. This is purely a JupyterLite-side change — the Tauri host is unaware of the language.

#### Additional WASM Kernels

New WASM-based kernels can be added to the JupyterLite deployment without touching the Tauri host:

| Language | WASM Runtime | Notes |
|---|---|---|
| R | [webR](https://docs.r-wasm.org/webr/) | R compiled to WASM, similar architecture to Pyodide |
| Ruby | [ruby.wasm](https://ruby.github.io/ruby.wasm/) | CRuby compiled to WASM |
| Lua | [Wasmoon](https://github.com/nicolo-ribaudo/wasmoon) | Lightweight WASM Lua |
| SQLite | [sql.js](https://github.com/sql-js/sql.js) | SQLite in WASM, already common in JupyterLite |

Each would be packaged as a JupyterLite kernel extension. The risk profile is identical to Pyodide: in-browser, same origin, no OS-level isolation.

### Tier 2: Host-Managed Native Kernels

For true native execution (real Python, Node.js, etc.) the Tauri host needs to:

1. **Spawn and manage kernel processes** — Launch an interpreter, wire stdin/stdout/stderr.
2. **Implement the Jupyter kernel protocol** — Or use an existing kernel gateway.
3. **Expose a kernel API to the WebView** — New Tauri commands for kernel lifecycle.

#### Architecture

```
Tauri Host (Rust)
├── KernelManager
│   ├── spawn_kernel(language) → KernelId
│   ├── execute(KernelId, code) → Stream<Output>
│   ├── interrupt(KernelId)
│   └── shutdown(KernelId)
│
├── Kernel Process: python -m ipykernel_launcher
├── Kernel Process: node --experimental-vm-modules
├── Kernel Process: deno run
└── ...
```

New Tauri commands:

```rust
#[command]
async fn spawn_kernel(language: String) -> Result<KernelId, String>;

#[command]
async fn kernel_execute(kernel_id: String, code: String) -> Result<(), String>;

#[command]
async fn kernel_interrupt(kernel_id: String) -> Result<(), String>;

#[command]
async fn kernel_shutdown(kernel_id: String) -> Result<(), String>;
```

The frontend would need a JupyterLite kernel extension that delegates execute requests to the Tauri host instead of running code in-browser.

**Risk:** Native kernel processes run with the full privileges of the user. There is no isolation beyond standard Unix process boundaries. A malicious notebook could read/write arbitrary files, make network connections, install software, etc.

### Tier 3: Container-Isolated Kernels

This is the highest-isolation option. Kernel code runs inside a container, limiting filesystem access, network access, and resource usage.

---

## Part 3 — Container-Based Execution

### Container Runtime Abstraction

Similar to the existing provider abstraction (`RepoProvider` / `PublishProvider`), container-based execution should be built on a trait:

```rust
/// Trait for container runtimes that can run kernel processes.
#[allow(async_fn_in_trait)]
pub trait ContainerRuntime {
    /// Check if this runtime is available on the system.
    async fn is_available(&self) -> bool;

    /// Create and start a container for a kernel session.
    async fn create_container(
        &self,
        params: &ContainerParams,
    ) -> Result<ContainerInfo, ContainerError>;

    /// Execute code inside a running container.
    async fn exec_in_container(
        &self,
        container_id: &str,
        command: &[String],
    ) -> Result<ExecOutput, ContainerError>;

    /// Stop and remove a container.
    async fn destroy_container(
        &self,
        container_id: &str,
    ) -> Result<(), ContainerError>;

    /// Open an interactive terminal session to a container.
    async fn attach_terminal(
        &self,
        container_id: &str,
        shell: &str,
    ) -> Result<TerminalSession, ContainerError>;
}

/// Parameters for creating a container.
pub struct ContainerParams {
    /// Container image (e.g., "jupyter/minimal-notebook:latest")
    pub image: String,
    /// Directories to mount (read-only or read-write)
    pub mounts: Vec<MountSpec>,
    /// Resource limits
    pub limits: ResourceLimits,
    /// Network mode ("none", "bridge", "host")
    pub network_mode: String,
    /// Environment variables
    pub env: Vec<(String, String)>,
}

pub struct ResourceLimits {
    pub memory_mb: u64,
    pub cpu_shares: u64,
    pub timeout_seconds: u64,
}
```

### Container Runtimes

#### Apple Containers (macOS native)

Apple Containers, introduced in macOS, provide lightweight Linux VMs using the Virtualization.framework. They are similar to Docker Desktop's LinuxKit VM but are a first-party Apple technology.

**Implementation path:**

1. Create `src-tauri/src/containers/apple.rs` implementing `ContainerRuntime`.
2. Use the `apple-containers` CLI tool (`container`) or the Virtualization.framework Swift bindings (via a helper process or FFI).
3. Apple Containers run Linux images — kernel images would be standard OCI images.
4. Mount the workspace directory (read-only) into the container for notebook file access.

```rust
pub struct AppleContainerRuntime;

impl ContainerRuntime for AppleContainerRuntime {
    async fn is_available(&self) -> bool {
        // Check for macOS 26+ and `container` CLI
        which::which("container").is_ok()
    }

    async fn create_container(&self, params: &ContainerParams)
        -> Result<ContainerInfo, ContainerError>
    {
        // container run --name <id> --memory <limit> <image>
        let id = uuid::Uuid::new_v4().to_string();
        let mut args = vec!["run", "--name", &id, "--detach"];

        for mount in &params.mounts {
            // --mount type=bind,source=<host>,target=<container>,readonly
            args.push("--mount");
            // ...
        }

        args.push(&params.image);
        run_cli("container", &args).await?;
        Ok(ContainerInfo { id, runtime: "apple".into() })
    }

    // ...
}
```

**Pros:** Native to macOS, no third-party daemon, lightweight, fast boot.
**Cons:** macOS 26+ only, Linux guests only, newer ecosystem.

#### Docker Desktop

The most widely-used container runtime. Docker Desktop runs a LinuxKit VM on macOS with the Docker Engine inside it.

**Implementation path:**

1. Create `src-tauri/src/containers/docker.rs` implementing `ContainerRuntime`.
2. Shell out to the `docker` CLI (like the existing git operations in `git/ops.rs`).
3. Use `docker run`, `docker exec`, `docker rm` for lifecycle management.

```rust
pub struct DockerRuntime;

impl ContainerRuntime for DockerRuntime {
    async fn is_available(&self) -> bool {
        run_cli("docker", &["info"]).await.is_ok()
    }

    async fn create_container(&self, params: &ContainerParams)
        -> Result<ContainerInfo, ContainerError>
    {
        let id = uuid::Uuid::new_v4().to_string();
        let mut args = vec![
            "run", "-d",
            "--name", &id,
            "--memory", &format!("{}m", params.limits.memory_mb),
            "--network", &params.network_mode,
        ];
        for (k, v) in &params.env {
            args.extend(["--env", &format!("{k}={v}")]);
        }
        for mount in &params.mounts {
            args.extend(["-v", &mount.to_docker_arg()]);
        }
        args.push(&params.image);
        args.push("sleep"); args.push("infinity"); // keep alive
        run_cli("docker", &args).await?;
        Ok(ContainerInfo { id, runtime: "docker".into() })
    }

    // ...
}
```

**Pros:** Ubiquitous, mature, huge image ecosystem, cross-platform.
**Cons:** Requires Docker Desktop license (commercial use), heavy VM overhead.

#### Podman

A daemonless, rootless container runtime. API-compatible with Docker.

**Implementation path:**

1. Create `src-tauri/src/containers/podman.rs` implementing `ContainerRuntime`.
2. Nearly identical to the Docker implementation — swap `docker` → `podman` in CLI calls.
3. Podman on macOS uses `podman machine` (a QEMU or Apple Virtualization VM).

```rust
pub struct PodmanRuntime;

impl ContainerRuntime for PodmanRuntime {
    async fn is_available(&self) -> bool {
        run_cli("podman", &["info"]).await.is_ok()
    }
    // CLI args are Docker-compatible
}
```

**Pros:** No daemon, rootless by default, open source, no license concerns.
**Cons:** Slightly less polished on macOS, needs `podman machine` running.

#### Cloudflare Containers / Sandboxes

Cloudflare provides two relevant primitives:

- **Containers** — Full OCI containers running on Cloudflare's edge network (recently launched). Each container gets a dedicated IP, runs for the session duration, and is destroyed afterward.
- **Workers** — V8 isolates for JavaScript/TypeScript/WASM execution. Extremely fast cold start (~0ms), strong isolation, but no filesystem or arbitrary process spawning.

**Implementation path:**

1. Create `src-tauri/src/containers/cloudflare.rs`.
2. For **Containers**: Use the Cloudflare API to provision a container, exec commands via the Cloudflare tunnel/API, and tear down on session end. Requires a Cloudflare account and the Containers beta.
3. For **Workers/Sandboxes** (lightweight JS/WASM): Deploy a Worker that accepts code execution requests and returns outputs. This is closer to a serverless function than a container.

```rust
pub struct CloudflareContainerRuntime {
    api_token: String,
    account_id: String,
}

impl ContainerRuntime for CloudflareContainerRuntime {
    async fn is_available(&self) -> bool {
        // Validate API token and account access
        validate_cf_api(&self.api_token, &self.account_id).await.is_ok()
    }

    async fn create_container(&self, params: &ContainerParams)
        -> Result<ContainerInfo, ContainerError>
    {
        // POST /accounts/{account_id}/containers
        let client = reqwest::Client::new();
        let resp = client
            .post(format!(
                "https://api.cloudflare.com/client/v4/accounts/{}/containers",
                self.account_id
            ))
            .bearer_auth(&self.api_token)
            .json(&serde_json::json!({
                "image": params.image,
                "memory_mb": params.limits.memory_mb,
            }))
            .send().await?;
        // ...
        Ok(ContainerInfo { id, runtime: "cloudflare".into() })
    }
    // ...
}
```

**Pros:** No local VM needed, remote execution, strong isolation, global edge network.
**Cons:** Requires internet, latency for I/O-heavy workloads, Cloudflare account/billing, API still evolving.

### Runtime Selection and Registry

The app needs a way to discover available runtimes and let the user choose:

```rust
pub struct ContainerRegistry {
    runtimes: Vec<Box<dyn ContainerRuntime + Send + Sync>>,
}

impl ContainerRegistry {
    pub fn new() -> Self {
        Self {
            runtimes: vec![
                Box::new(AppleContainerRuntime),
                Box::new(DockerRuntime),
                Box::new(PodmanRuntime),
                // Cloudflare requires config, added separately
            ],
        }
    }

    /// Probe all runtimes and return which ones are available.
    pub async fn available_runtimes(&self) -> Vec<&str> {
        let mut available = vec![];
        for rt in &self.runtimes {
            if rt.is_available().await {
                available.push(rt.name());
            }
        }
        available
    }
}
```

New Tauri commands:

```rust
#[command]
async fn list_container_runtimes() -> Result<Vec<RuntimeInfo>, String>;

#[command]
async fn start_container_kernel(
    runtime: String,
    image: String,
    workspace_id: Option<String>,
) -> Result<KernelSession, String>;

#[command]
async fn start_container_terminal(
    runtime: String,
    image: String,
    workspace_id: Option<String>,
) -> Result<TerminalSession, String>;
```

---

## Part 4 — Terminal Support

Terminals are a separate concern from kernels. A kernel executes code cells and produces structured output (text, images, errors). A terminal provides an interactive shell session.

### Terminal Architecture

```
Frontend (JupyterLite)                 Tauri Host
┌──────────────┐                 ┌─────────────────────┐
│  Terminal    │   ← IPC PTY →   │  TerminalManager    │
│  Widget      │     events      │  ┌───────────────┐  │
│  (xterm.js)  │                 │  │ Container     │  │
│              │                 │  │ Runtime       │  │
│              │                 │  │  └─ /bin/bash │  │
│              │                 │  └───────────────┘  │
└──────────────┘                 └─────────────────────┘
```

The Tauri host would:

1. Create a container using the selected runtime.
2. Attach a PTY to the container's shell process.
3. Stream PTY I/O over Tauri events (not request/response — use `app.emit()` and event listeners).
4. The frontend renders the stream in an xterm.js terminal widget.

### Tauri Commands for Terminals

```rust
#[command]
async fn open_terminal(
    runtime: String,
    image: String,
    workspace_id: Option<String>,
    shell: Option<String>,  // defaults to "/bin/bash"
) -> Result<TerminalId, String>;

#[command]
async fn terminal_input(
    terminal_id: String,
    data: String,  // raw bytes from xterm.js
) -> Result<(), String>;

#[command]
async fn terminal_resize(
    terminal_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String>;

#[command]
async fn close_terminal(
    terminal_id: String,
) -> Result<(), String>;
```

Output flows via Tauri events:

```rust
// In the terminal I/O loop:
app.emit("terminal-output", TerminalOutputEvent {
    terminal_id: id.clone(),
    data: output_bytes,
}).unwrap();
```

---

## Part 5 — Module Layout

Proposed file structure for the container and terminal additions:

```
src-tauri/src/
├── containers/
│   ├── mod.rs              # ContainerRuntime trait, ContainerRegistry
│   ├── types.rs            # ContainerParams, ResourceLimits, etc.
│   ├── apple.rs            # Apple Containers implementation
│   ├── docker.rs           # Docker Desktop implementation
│   ├── podman.rs           # Podman implementation
│   └── cloudflare.rs       # Cloudflare Containers/Workers
├── kernels/
│   ├── mod.rs              # KernelManager, KernelSession
│   ├── types.rs            # KernelId, ExecuteRequest, Output
│   └── container_kernel.rs # Kernel backed by a container runtime
├── terminal/
│   ├── mod.rs              # TerminalManager, TerminalSession
│   └── container_pty.rs    # PTY relay to container shell
├── container_commands.rs   # Tauri commands for container management
├── kernel_commands.rs      # Tauri commands for kernel lifecycle
└── terminal_commands.rs    # Tauri commands for terminal I/O
```

Frontend additions:

```
src/
├── containers/
│   ├── types.ts            # RuntimeInfo, ContainerSession
│   └── api.ts              # Tauri command wrappers
├── kernels/
│   ├── types.ts            # KernelSession, ExecuteResult
│   └── api.ts              # Tauri command wrappers
└── terminal/
    ├── types.ts            # TerminalSession, TerminalEvent
    └── api.ts              # Tauri command wrappers
```

---

## Part 6 — Security Comparison

| Execution environment | Process isolation | Filesystem isolation | Network isolation | Resource limits | Escape risk |
|---|---|---|---|---|---|
| JupyterLite JS kernel | None (same page) | None (same IndexedDB) | CORS only | None | Browser engine bug |
| JupyterLite Pyodide | WASM linear memory | None (same IndexedDB) | CORS only | None | WASM engine bug |
| Native kernel (Tier 2) | OS process boundary | User-level access | Full network | ulimit | Process escape |
| Apple Container | VM boundary | Image rootfs only | Configurable | cgroup-like | VM escape |
| Docker | Linux namespace + cgroup | Image rootfs + mounts | Configurable | cgroup | Container escape |
| Podman (rootless) | User namespace + cgroup | Image rootfs + mounts | Configurable | cgroup | Container escape (reduced surface) |
| Cloudflare Container | Remote VM + network | Remote rootfs | Cloudflare network | Cloudflare-managed | Remote escape (Cloudflare's problem) |
| Cloudflare Worker | V8 isolate | None (no FS) | Fetch API only | CPU time + memory | V8 isolate escape |

---

## Part 7 — Implementation Priorities

A suggested phased rollout:

1. **Phase 1 — Container runtime trait + Docker/Podman** — Most users already have Docker. Implement the trait, Docker runtime, and Podman runtime. Add terminal support.
2. **Phase 2 — Apple Containers** — Add native macOS container support once the Apple Containers API stabilizes.
3. **Phase 3 — Container-backed kernels** — Use the container runtime to run real Jupyter kernels (`ipykernel`, `tslab`, etc.) inside containers, wiring them to JupyterLite's kernel protocol.
4. **Phase 4 — Cloudflare** — Add remote container execution for users who prefer not to run containers locally.
5. **Phase 5 — Additional in-browser kernels** — Add TypeScript transpilation, webR, or other WASM kernels as JupyterLite extensions (no Tauri host changes needed).
