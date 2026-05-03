# Execution Design & Implementation

How code runs in Wiki3 for Mac — today and in the future.

---

## Part 1 — Purpose and Mental Model

Wiki3 is a local-first desktop app for editing, running, and publishing **knowledge gardens** — JupyterLite-based sites (notebooks, documents, data, and AI agents) managed as git repos. The model is analogous to [Quartz](https://quartz.jzhao.xyz/philosophy): content lives in a repo, is edited locally, and is published as a static site. Wiki3 extends this to include **executable content** (notebooks with kernels, LLMs, and agents) and **container-based environments**.

The key primitives are:

| Primitive | Role |
|---|---|
| **Git repo** | Unit of content, identity, versioning, and deployment. Content enters and exits all environments via git. |
| **Identity** | Who owns/authored the repo and its execution. Today: GitHub user. Future: Radicle cryptographic identity, AT Protocol DID. |
| **Kernel** | Executes notebook cells. In-browser (JupyterLite) or container-backed. |
| **Terminal** | Interactive shell session inside a container. |
| **Container** | Isolated Linux environment for kernels and terminals. Content is mapped in from git repos. |
| **Site** | Published static output of a repo, served via GitHub Pages, Cloudflare, or other hosts. |

The desktop app's job is to make all of this work locally without relying on third-party services that a conventional browser requires.

---

## Part 2 — Current Architecture

### How the Desktop App Runs Code

Wiki3 for Mac is a **Tauri 2** desktop app. It opens a WebView window pointing at a JupyterLite site and acts as a permission-gated host. It does **not** launch Python interpreters or spawn Jupyter kernel processes — kernels run inside the WebView's JupyterLite (Pyodide WASM, JavaScript, AI SDK Chat).

For *local-preview* serving (the `Serve` button on a wiki card), the app does run an Apple Container that hosts the JupyterLite static build. The design and failure modes of that path are documented separately in [networking.md](./networking.md). The container hosts the *site*, not the kernel — kernels are still in-WebView.

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
│  │   │  ┌──────────┐  ┌───────────────┐  │   │   │
│  │   │  │ Notebook │  │ Desktop Ext.  │  │   │   │
│  │   │  │ Editor   │  │ (bridge.ts)   │  │   │   │
│  │   │  └────┬─────┘  └───────────────┘  │   │   │
│  │   │       │                           │   │   │
│  │   │  ┌────┴─────────────────────┐     │   │   │
│  │   │  │  JupyterLite Kernels     │     │   │   │
│  │   │  │  • Pyodide (WASM Python) │     │   │   │
│  │   │  │  • JavaScript            │     │   │   │
│  │   │  │  • AI SDK Chat (LLM)     │     │   │   │
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

#### Git-Based Content Management

The Tauri host already manages git repos via CLI operations in `git/ops.rs`:

- Clone (with transient token-authenticated URLs)
- Status, commit, push
- Remote management (origin, upstream)
- Branch operations

Workspaces (`workspace/types.rs`) represent a local repo connected to a remote provider. This is the foundation for mapping content into execution environments.

#### What the Tauri Host Does NOT Do (Today)

- Does not launch Python, Node.js, or any interpreter process for kernel execution.
- Does not manage Jupyter kernel lifecycle, restart, or interrupt.
- Does not provide filesystem access to kernel code.
- Does not run *kernel* containers — but it **does** run *site-preview* containers (Apple Container running JupyterLite via `Serve`); see [networking.md](./networking.md).
- Does not sandbox kernel code beyond the WebView boundary.

---

### JupyterLite: How It Works

JupyterLite is a full JupyterLab distribution that runs entirely in the browser. Wiki3 uses a remotely-hosted JupyterLite deployment (served from wiki3.ai or GitHub Pages).

#### Key Components

**JupyterLite Core** — A static build of JupyterLab compiled to run without a server. It uses Service Workers and IndexedDB to emulate the Jupyter server API inside the browser.

**Extensions** — Standard JupyterLab extensions, bundled at build time. The Wiki3 desktop integration extension (`src/lib/`) is one of these — it hooks into JupyterLab's lifecycle to gate execution based on the host's permission state. Third-party kernel extensions (like the AI SDK Chat kernel) are also bundled into the site build.

**Notebooks** — `.ipynb` files stored in the browser's IndexedDB (the emulated "filesystem"). They persist across page reloads and app launches because IndexedDB state is scoped to the origin and survives WebView teardown.

**Kernels** — JupyterLite provides in-browser kernel implementations:

| Kernel | Runtime | How it works |
|---|---|---|
| **Pyodide** | WebAssembly | CPython compiled to WASM via Emscripten. Runs a full Python 3.x interpreter inside the browser. Packages are loaded from PyPI via micropip. I/O is emulated. |
| **JavaScript** | Browser JS engine | Executes code using the page's own JavaScript runtime (`eval` or Web Worker). Has direct access to browser APIs. |
| **AI SDK Chat** | Browser JS + network | JupyterLite kernel extension (`@wiki3-ai/ai-sdk-chat-kernel`) using Vercel AI SDK. Supports local LLM providers (Built-in AI / Prompt API, WebLLM via WebGPU, Transformers.js via WASM) and cloud providers (OpenAI, Anthropic, Google). Uses magic commands (`%chat provider`, `%chat model`, etc.) for configuration. |
| **WebLLM** | WebGPU + WASM | Local LLM inference using WebGPU. Requires a WebGPU-enabled browser/WebView. |

All kernels run **in the same origin and process** as the JupyterLite UI. There is no kernel process, no ZeroMQ, no separate address space. The "kernel" is a JavaScript class that receives execute requests and produces outputs, using `postMessage` to a Web Worker at most.

#### Known Issues with Current Kernels

**AI SDK Chat Kernel** — Currently failing in the Wiki3 WebView with a stderr error pointing to `@wiki3-ai/ai-sdk-chat-kernel/index.js:117851:30`. This is likely caused by one or more of:

1. **Missing WebView APIs** — WKWebView on macOS does not expose the Chrome/Edge Prompt API (`window.ai`), so `built-in-ai/core` fails. The kernel's auto-selection fallback chain then tries WebLLM (needs WebGPU, see below) and Transformers.js (needs certain WASM features and potentially more relaxed CORS). If all three local providers fail, the kernel may throw during initialization rather than gracefully falling back to a "no provider configured" state.

2. **CSP restrictions** — The app's CSP includes `'unsafe-eval'` and `'unsafe-inline'` for scripts but restricts `connect-src` to specific domains. The AI SDK kernel may need to fetch model configs or weights from domains not in the allowlist (e.g., Hugging Face CDN, WebLLM model repos). The current CSP allows `https://cdn.jsdelivr.net`, `https://pypi.org`, `https://files.pythonhosted.org`, and `https://api.github.com`, but not `https://huggingface.co` or model CDN domains.

3. **Worker/SharedArrayBuffer** — Some AI inference libraries require `SharedArrayBuffer`, which needs `Cross-Origin-Opener-Policy: same-origin` and `Cross-Origin-Embedder-Policy: require-corp` headers. WKWebView may not set these, and the remote site may not serve them.

**WebLLM Kernel** — Reports "WebGPU is unavailable." WKWebView on macOS does not currently expose the WebGPU API (`navigator.gpu`). Safari has WebGPU support behind a feature flag, but WKWebView embedded in apps does not inherit Safari's feature flags. This is a platform limitation — WebGPU in WKWebView requires Apple to enable it for third-party app WebViews, or the app to use a different WebView engine. This is beyond the next milestone but worth tracking.

**Remediation paths for AI SDK Chat kernel:**

- Add Hugging Face domains to the CSP `connect-src` allowlist
- Ensure the kernel's fallback chain handles missing APIs gracefully (report "no local provider available, configure a cloud provider" rather than throwing)
- For cloud providers (OpenAI, Anthropic, Google): add their API domains to `connect-src`
- Long term: container-backed kernels eliminate these WebView limitations entirely

#### Isolation Properties

| Property | Pyodide (WASM) | JavaScript | AI SDK Chat |
|---|---|---|---|
| Separate OS process | No | No | No |
| Separate thread | Web Worker (optional) | Web Worker (optional) | Web Worker (optional) |
| Memory isolation | WASM linear memory | None — shares page JS | Shares page JS |
| Can access DOM | Via JS interop bridge | Yes, directly | Yes, directly |
| Can call `fetch()` | Yes (subject to CORS/CSP) | Yes | Yes (core functionality) |
| Can access IndexedDB | Yes | Yes | Yes |
| Can see `__TAURI_INTERNALS__` | Via JS interop | Yes, directly | Yes, directly |
| Can exhaust CPU/memory | Yes | Yes | Yes |

**Key point:** WASM provides language-level sandboxing (no raw pointer access to the host process) but does not provide OS-level isolation. All kernel types live inside the same WebView, have the same network access, and are bounded only by the browser/WebView sandbox.

---

## Part 3 — Git Repos as the Content Fabric

### The Central Role of Git

In Wiki3, **git repos are the primary means of getting content into and out of execution environments**. This applies to:

| Use case | Git's role |
|---|---|
| Editing a knowledge garden | Content is in a local clone of a repo |
| Running notebooks | Notebook files come from the repo |
| Publishing a site | Built output is pushed to the repo (gh-pages branch or /docs folder) |
| Provisioning a container | The repo (at a specific commit) defines what content is mounted |
| Defining an environment | The repo can contain a Dockerfile, devcontainer.json, nix flake, or guix manifest |
| Collaborating | Fork, branch, merge, pull request — all git |
| Reproducibility | A repo + commit SHA = exact, immutable content snapshot |

This is the same paradigm used by:

- **Nix/Guix** — Environment definitions are content-addressed and reproducible from source. A flake.nix or guix manifest in the repo fully specifies the environment.
- **Cloudflare Artifacts** — "Versioned file trees behind a git-compatible interface." One repo per agent, user, branch, or task. Fork from a shared baseline, diff/merge later.
- **Dev Containers** — `.devcontainer/devcontainer.json` in the repo defines the container image and configuration.

### Content-Addressed Environments

When mapping content into an execution environment, we specify it by **repo identity + commit**:

```
ContentRef {
    // Where the content lives
    repo: RepoIdentity,       // GitHub URL, Radicle URN, or Cloudflare Artifact ID
    commit: Option<String>,    // Specific commit SHA (None = HEAD)
    branch: Option<String>,    // Branch name (used if commit is None)
    path: Option<String>,      // Subdirectory within the repo

    // How to mount it
    mount_point: String,       // Where to mount inside the container
    mount_mode: MountMode,     // ReadOnly, ReadWrite, Overlay (CoW)
}

enum RepoIdentity {
    GitHub { owner: String, repo: String },
    Radicle { urn: String },              // rad:z3gqcJUoA1n9HaHKufZs5FCSGazv5
    CloudflareArtifact { repo_id: String },
    Local { path: String },               // Local filesystem path
}
```

A container environment is then defined as one or more `ContentRef` mappings:

```
EnvironmentSpec {
    image: String,                   // OCI image for the container
    content: Vec<ContentRef>,        // Repos/commits to mount
    env_definition: Option<ContentRef>, // Repo containing Dockerfile/devcontainer.json/nix flake
    env_vars: Vec<(String, String)>,
    resource_limits: ResourceLimits,
}
```

This maps directly to Cloudflare's model: Containers run OCI images with Artifacts providing git-based versioned storage. The same `EnvironmentSpec` works for local containers (Apple Containers, Docker, Podman) and remote containers (Cloudflare).

### Mutable vs Immutable Content

| Mount mode | Semantics | Use case |
|---|---|---|
| **ReadOnly** | Exact commit, no writes. Content-addressed. | Reference data, dependencies, shared baselines |
| **ReadWrite** | Clone at commit, mutations allowed, can push back | Active editing, notebook execution with state |
| **Overlay** | Read-only base + writable CoW layer | Experiments, agent execution, sandboxed trials |

The Overlay mode is particularly important for agents: fork from a shared baseline, let the agent work in its own writable layer, then diff or merge the results. This is exactly how Cloudflare Artifacts describes its multi-repo workflow: "Isolate work in separate repos or branches for safer parallel execution. Fork from a shared baseline and diff or merge the results later."

---

## Part 4 — Identity and Provenance

### Current: GitHub Identity

Today, identity is a GitHub user/org + PAT. The `GitHubAuth` module stores tokens in the OS keychain and injects them transiently for git operations.

### Future: Radicle (Cryptographic Git Identity)

[Radicle](https://radicle.xyz/) is a peer-to-peer code collaboration stack built on git. It provides:

- **Cryptographic identity** — Each user and repo has a public-key identity (e.g., `rad:z3gqcJUoA1n9HaHKufZs5FCSGazv5`). All data is signed.
- **Decentralized replication** — Repos are replicated across peers via a gossip protocol. No single entity controls the network.
- **Local-first** — Always-available functionality offline. Users own their data.
- **Collaborative Objects (COBs)** — Issues, patches, discussions stored as git objects. Extensible to arbitrary collaboration primitives.
- **Git-native** — Radicle storage is git. A Radicle repo *is* a git repo with additional signed metadata.
- **Modular stack** — CLI, web interface, TUI, backed by Radicle Node and HTTP Daemon. Any part can be swapped.

#### Why Radicle Matters for Wiki3

1. **Repo identity without GitHub** — A knowledge garden can be identified by its Radicle URN rather than a GitHub owner/repo pair. This enables censorship-resistant hosting and true ownership.

2. **Agent identity** — An AI agent operating on a repo can have its own Radicle identity. Its commits are signed with its key. Provenance is cryptographically verifiable: "this commit was made by agent X in container Y using content from repo Z at commit W."

3. **Container provenance** — When a container environment is specified by a Radicle URN + commit, the content is authenticated end-to-end. No one can tamper with what the container sees.

4. **Peer-to-peer sync** — Repos can sync between the desktop app and container environments without going through GitHub. The Radicle node can run on the host, in a container, or on a remote peer.

5. **Collaborative Objects for execution** — COBs could be extended to represent execution sessions, agent task assignments, or container lifecycle events as git objects.

#### Integration Path

```rust
enum RepoIdentity {
    GitHub { owner: String, repo: String },
    Radicle { urn: String },  // rad:z3gqcJUoA1n9HaHKufZs5FCSGazv5
    // ...
}

/// Radicle identity for a user or agent
struct RadicleIdentity {
    /// Node ID (public key)
    node_id: String,     // e.g., "z6Mkf..."
    /// Display alias
    alias: Option<String>,
}
```

The existing `RepoProvider` trait can be extended with a `RadicleRepoProvider` that uses the Radicle CLI (`rad`) or HTTP daemon for repo operations. Radicle repos are standard git repos, so the existing `git/ops.rs` module works for clone, commit, push — the difference is the remote URL format and authentication (SSH key-based, not token-based).

#### AT Protocol (Future)

[AT Protocol](https://atproto.com/) (Bluesky's protocol) provides decentralized identity (DIDs) and content-addressed data. Its relevance to Wiki3 is in user identity and social layer (publishing, discovery, reputation). This is noted here for future reference but is not part of the next milestone.

---

## Part 5 — Adding More Kernel Execution Options

There are three tiers of kernel isolation, each with increasing capability and complexity.

### Tier 1: In-Browser Kernels (Current)

These are the existing JupyterLite kernels. No changes needed to the Tauri host.

#### JS/TS Kernel (exists today)

JupyterLite's JavaScript kernel already supports ES module syntax. To add TypeScript support:

1. Bundle a TypeScript transpiler (e.g., `typescript` compiler or `sucrase`) into the JupyterLite build.
2. Create a JupyterLite kernel extension that transpiles `.ts` cell contents to JS before execution.
3. This is purely a JupyterLite-side change — the Tauri host is unaware of the language.

#### AI SDK Chat Kernel (exists, needs fixes)

The `@wiki3-ai/ai-sdk-chat-kernel` is a JupyterLite kernel extension providing LLM chat via Vercel AI SDK. It supports multiple providers with auto-selection fallback:

1. `built-in-ai/core` — Chrome/Edge Prompt API (Gemini Nano, Phi-4 Mini)
2. `built-in-ai/webllm` — WebLLM local inference via WebGPU
3. `built-in-ai/transformers` — Transformers.js local inference via WASM

**Immediate fix needed:** The kernel throws during initialization when running in WKWebView because none of the three local providers are available (no Prompt API, no WebGPU, and potentially CORS/CSP issues for Transformers.js model downloads). The fix should be in the kernel extension itself:

- Gracefully handle all-providers-unavailable without throwing
- Default to awaiting cloud provider configuration (`%chat provider openai --key ...`)
- Report clear status: "No local AI providers available in this environment. Use `%chat provider <name> --key <key>` to configure a cloud provider."

**CSP fix needed in wiki3-app:** Add these domains to `connect-src` in `tauri.conf.json`:

- `https://huggingface.co` and `https://*.huggingface.co` — for Transformers.js model downloads
- `https://api.openai.com` — for OpenAI provider
- `https://api.anthropic.com` — for Anthropic provider
- `https://generativelanguage.googleapis.com` — for Google provider

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
│   ├── spawn_kernel(language, content_refs) → KernelId
│   ├── execute(KernelId, code) → Stream<Output>
│   ├── interrupt(KernelId)
│   └── shutdown(KernelId)
│
├── Kernel Process: python -m ipykernel_launcher  (working dir = repo clone)
├── Kernel Process: node --experimental-vm-modules
├── Kernel Process: deno run
└── ...
```

New Tauri commands:

```rust
#[command]
async fn spawn_kernel(
    language: String,
    workspace_id: Option<String>,  // repo context for the kernel
) -> Result<KernelId, String>;

#[command]
async fn kernel_execute(kernel_id: String, code: String) -> Result<(), String>;

#[command]
async fn kernel_interrupt(kernel_id: String) -> Result<(), String>;

#[command]
async fn kernel_shutdown(kernel_id: String) -> Result<(), String>;
```

The frontend would need a JupyterLite kernel extension that delegates execute requests to the Tauri host instead of running code in-browser.

**Risk:** Native kernel processes run with the full privileges of the user. There is no isolation beyond standard Unix process boundaries.

### Tier 3: Container-Isolated Kernels

This is the highest-isolation option. Kernel code runs inside a container, with content from git repos mapped in as mounts. See Part 6.

---

## Part 6 — Container-Based Execution

### Container Runtime Abstraction

Similar to the existing provider abstraction (`RepoProvider` / `PublishProvider`), container-based execution should be built on a trait:

```rust
/// Trait for container runtimes that can run kernel processes and terminals.
#[allow(async_fn_in_trait)]
pub trait ContainerRuntime: Send + Sync {
    /// Human-readable name of this runtime.
    fn name(&self) -> &str;

    /// Check if this runtime is available on the system.
    async fn is_available(&self) -> bool;

    /// Create and start a container from an environment spec.
    async fn create_container(
        &self,
        spec: &EnvironmentSpec,
    ) -> Result<ContainerInfo, ContainerError>;

    /// Execute a command inside a running container.
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

/// Specifies a complete container environment.
pub struct EnvironmentSpec {
    /// OCI image for the container
    pub image: String,
    /// Git repos/commits to mount into the container
    pub content: Vec<ContentRef>,
    /// Optional: repo containing Dockerfile, devcontainer.json, nix flake
    pub env_definition: Option<ContentRef>,
    /// Resource limits
    pub limits: ResourceLimits,
    /// Network mode ("none", "bridge", "host")
    pub network_mode: String,
    /// Environment variables
    pub env: Vec<(String, String)>,
}

/// Reference to git content to mount into a container.
pub struct ContentRef {
    /// Repository identity
    pub repo: RepoIdentity,
    /// Specific commit (None = HEAD of branch)
    pub commit: Option<String>,
    /// Branch (used if commit is None)
    pub branch: Option<String>,
    /// Subdirectory within the repo
    pub path: Option<String>,
    /// Where to mount inside the container
    pub mount_point: String,
    /// Read-only, read-write, or overlay
    pub mount_mode: MountMode,
}

pub enum MountMode {
    ReadOnly,
    ReadWrite,
    Overlay,  // CoW: read-only base + writable upper layer
}

pub struct ResourceLimits {
    pub memory_mb: u64,
    pub cpu_shares: u64,
    pub timeout_seconds: u64,
}
```

### Git Content Provisioning

Before creating a container, the host must **resolve `ContentRef` entries to local paths**:

```rust
/// Resolves a ContentRef to a local directory suitable for bind-mounting.
async fn resolve_content(
    content_ref: &ContentRef,
    workspace_manager: &WorkspaceManager,
) -> Result<ResolvedMount, ContentError> {
    match &content_ref.repo {
        RepoIdentity::Local { path } => {
            // Already local — use directly
            Ok(ResolvedMount { host_path: path.clone(), .. })
        }
        RepoIdentity::GitHub { owner, repo } => {
            // Check if we have a workspace for this repo
            // If not, clone (read-only: shallow clone at commit; read-write: full clone)
            // If commit is specified, checkout that commit
            let local = ensure_cloned(owner, repo, content_ref.commit.as_deref()).await?;
            Ok(ResolvedMount { host_path: local, .. })
        }
        RepoIdentity::Radicle { urn } => {
            // Clone via rad clone or local Radicle storage
            let local = ensure_rad_cloned(urn, content_ref.commit.as_deref()).await?;
            Ok(ResolvedMount { host_path: local, .. })
        }
        RepoIdentity::CloudflareArtifact { repo_id } => {
            // For local containers: clone via git (Artifacts speaks git)
            // For Cloudflare Containers: use ArtifactFS binding (no local clone needed)
            let local = ensure_artifact_cloned(repo_id, content_ref.commit.as_deref()).await?;
            Ok(ResolvedMount { host_path: local, .. })
        }
    }
}
```

### Container Runtimes

#### Apple Containers (macOS native)

Apple Containers provide lightweight Linux VMs using the Virtualization.framework. First-party Apple technology, introduced in macOS 26.

**Implementation path:**

1. Create `src-tauri/src/containers/apple.rs` implementing `ContainerRuntime`.
2. Use the `container` CLI tool or Virtualization.framework Swift bindings.
3. Apple Containers run standard OCI images — the same images work on Docker/Podman.
4. Bind-mount resolved `ContentRef` paths into the container.

```rust
pub struct AppleContainerRuntime;

impl ContainerRuntime for AppleContainerRuntime {
    fn name(&self) -> &str { "apple" }

    async fn is_available(&self) -> bool {
        run_cli("container", &["--version"]).await.is_ok()
    }

    async fn create_container(&self, spec: &EnvironmentSpec)
        -> Result<ContainerInfo, ContainerError>
    {
        let id = uuid::Uuid::new_v4().to_string();
        let mut args = vec!["run", "--name", &id, "--detach"];

        for mount in &spec.resolved_mounts {
            let mount_arg = format!(
                "type=bind,source={},target={},{}",
                mount.host_path, mount.container_path,
                if mount.readonly { "readonly" } else { "" }
            );
            args.extend(["--mount", &mount_arg]);
        }

        args.push(&spec.image);
        run_cli("container", &args).await?;
        Ok(ContainerInfo { id, runtime: "apple".into() })
    }
    // ...
}
```

**Pros:** Native to macOS, no third-party daemon, lightweight, fast boot.
**Cons:** macOS 26+ only, Linux guests only, newer ecosystem.

#### Docker Desktop

The most widely-used container runtime. Docker Desktop runs a LinuxKit VM on macOS.

**Implementation path:**

1. Create `src-tauri/src/containers/docker.rs` implementing `ContainerRuntime`.
2. Shell out to the `docker` CLI (like the existing git operations in `git/ops.rs`).
3. Use `docker run`, `docker exec`, `docker rm` for lifecycle management.

**Pros:** Ubiquitous, mature, huge image ecosystem, cross-platform.
**Cons:** Requires Docker Desktop license (commercial use), heavy VM overhead.

#### Podman

A daemonless, rootless container runtime. API-compatible with Docker.

**Implementation path:**

1. Create `src-tauri/src/containers/podman.rs` implementing `ContainerRuntime`.
2. Nearly identical to the Docker implementation — swap `docker` → `podman` in CLI calls.
3. Podman on macOS uses `podman machine` (a QEMU or Apple Virtualization VM).

**Pros:** No daemon, rootless by default, open source, no license concerns.
**Cons:** Slightly less polished on macOS, needs `podman machine` running.

#### Cloudflare Containers + Artifacts

Cloudflare provides two primitives that map directly to our model:

**Artifacts** — "Versioned storage that speaks Git." Create repositories programmatically, import existing repositories, hand off a URL to any standard git client. One repo per agent, user, branch, or task. Fork from a shared baseline, diff/merge later. Accessible from Workers, the REST API, and git clients.

**Containers** — Full OCI containers running on Cloudflare's edge network. Controlled by a Worker script. Instances spin up on-demand and are destroyed after a configurable idle timeout. Containers can mount Artifacts via ArtifactFS for git-based content access without cloning.

```
┌────────────────────────────────────────────────┐
│  Cloudflare Edge                               │
│                                                │
│  ┌──────────────┐     ┌─────────────────────┐  │
│  │  Worker      │────▶│  Container          │  │
│  │  (routing)   │     │  ┌───────────────┐  │  │
│  │              │     │  │ ArtifactFS    │  │  │
│  └──────────────┘     │  │ (git content) │  │  │
│                       │  └───────────────┘  │  │
│  ┌──────────────┐     │  ┌───────────────┐  │  │
│  │  Artifacts   │────▶│  │ Kernel/Shell  │  │  │
│  │  (git repos) │     │  └───────────────┘  │  │
│  └──────────────┘     └─────────────────────┘  │
└────────────────────────────────────────────────┘
```

**Implementation path:**

1. Create `src-tauri/src/containers/cloudflare.rs` implementing `ContainerRuntime`.
2. For container lifecycle: use the Cloudflare Workers API or Wrangler CLI.
3. For content: push workspace content to Cloudflare Artifacts, which the container accesses via ArtifactFS. Since Artifacts speaks git, existing `git/ops.rs` operations work for push/pull.
4. For local development: Cloudflare provides local dev tooling for Containers.

```rust
pub struct CloudflareRuntime {
    api_token: String,
    account_id: String,
}

impl ContainerRuntime for CloudflareRuntime {
    fn name(&self) -> &str { "cloudflare" }

    async fn is_available(&self) -> bool {
        validate_cf_api(&self.api_token, &self.account_id).await.is_ok()
    }

    async fn create_container(&self, spec: &EnvironmentSpec)
        -> Result<ContainerInfo, ContainerError>
    {
        // 1. Ensure content is synced to Artifacts repos (git push)
        for content in &spec.content {
            self.ensure_artifact_synced(content).await?;
        }

        // 2. Create container with Artifact bindings
        let client = reqwest::Client::new();
        let resp = client
            .post(format!(
                "https://api.cloudflare.com/client/v4/accounts/{}/containers",
                self.account_id
            ))
            .bearer_auth(&self.api_token)
            .json(&serde_json::json!({
                "image": spec.image,
                "memory_mb": spec.limits.memory_mb,
                "artifacts": spec.content.iter().map(|c| {
                    serde_json::json!({
                        "repo_id": c.artifact_id(),
                        "mount_point": c.mount_point,
                        "commit": c.commit,
                    })
                }).collect::<Vec<_>>(),
            }))
            .send().await?;
        // ...
        Ok(ContainerInfo { id, runtime: "cloudflare".into() })
    }
    // ...
}
```

**Pros:** No local VM needed, remote execution, strong isolation, global edge, git-native content via Artifacts, scales to many concurrent agents.
**Cons:** Requires internet, latency for I/O-heavy workloads, Cloudflare account/billing, API still evolving.

#### Cloudflare Workers (V8 Isolates)

For lightweight JS/TS/WASM execution without a full container:

- Deploy a Worker that accepts code execution requests
- Extremely fast cold start (~0ms), strong V8 isolation
- No filesystem, but can access Artifacts via bindings
- Good for: AI agent execution, JS/TS kernels, WASM computation
- Not suitable for: arbitrary Linux programs, Python with native packages

This can be exposed as an additional kernel type rather than a `ContainerRuntime`.

### Runtime Selection and Registry

```rust
pub struct ContainerRegistry {
    runtimes: Vec<Box<dyn ContainerRuntime>>,
}

impl ContainerRegistry {
    pub fn new() -> Self {
        Self {
            runtimes: vec![
                Box::new(AppleContainerRuntime),
                Box::new(DockerRuntime),
                Box::new(PodmanRuntime),
                // Cloudflare requires config, added via register()
            ],
        }
    }

    /// Probe all runtimes and return which ones are available.
    pub async fn available_runtimes(&self) -> Vec<RuntimeInfo> {
        let mut available = vec![];
        for rt in &self.runtimes {
            if rt.is_available().await {
                available.push(RuntimeInfo {
                    name: rt.name().to_string(),
                    local: !matches!(rt.name(), "cloudflare"),
                });
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
    content_refs: Option<Vec<ContentRef>>,
) -> Result<KernelSession, String>;

#[command]
async fn start_container_terminal(
    runtime: String,
    image: String,
    workspace_id: Option<String>,
    content_refs: Option<Vec<ContentRef>>,
) -> Result<TerminalSession, String>;
```

---

## Part 7 — Terminal Support

Terminals are a separate concern from kernels. A kernel executes code cells and produces structured output (text, images, errors). A terminal provides an interactive shell session.

### Terminal Architecture

```
Frontend (JupyterLite)                 Tauri Host
┌──────────────┐                ┌──────────────────────────┐
│  Terminal    │  ← IPC PTY →   │  TerminalManager         │
│  Widget      │    events      │  ┌────────────────────┐  │
│  (xterm.js)  │                │  │ Container          │  │
│              │                │  │  ┌──────────────┐  │  │
│              │                │  │  │ /workspace/  │  │  │
│              │                │  │  │ (git repo)   │  │  │
│              │                │  │  └──────────────┘  │  │
│              │                │  │  └─ /bin/bash      │  │
│              │                │  └────────────────────┘  │
└──────────────┘                └──────────────────────────┘
```

The Tauri host would:

1. Resolve `ContentRef` entries to local paths (clone repos if needed).
2. Create a container using the selected runtime, bind-mounting the resolved content.
3. Attach a PTY to the container's shell process.
4. Stream PTY I/O over Tauri events (not request/response — use `app.emit()` and event listeners).
5. The frontend renders the stream in an xterm.js terminal widget.

### Tauri Commands for Terminals

```rust
#[command]
async fn open_terminal(
    runtime: String,
    image: String,
    workspace_id: Option<String>,
    content_refs: Option<Vec<ContentRef>>,
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

### Git Integration in Terminals

Inside a container terminal, the user can run git commands against the mounted workspace. When the workspace is mounted read-write:

- `git status`, `git diff` — see local changes
- `git commit`, `git push` — push changes back (using transient token injection)
- The host can detect commits made inside the container and update workspace metadata

For Cloudflare Containers with Artifacts, git operations inside the container target the Artifact repo directly — no round-trip through the host.

---

## Part 8 — Module Layout

Proposed file structure for the container, kernel, identity, and terminal additions:

```
src-tauri/src/
├── containers/
│   ├── mod.rs              # ContainerRuntime trait, ContainerRegistry
│   ├── types.rs            # EnvironmentSpec, ContentRef, MountMode, ResourceLimits
│   ├── content.rs          # ContentRef resolution (git clone/checkout to local path)
│   ├── apple.rs            # Apple Containers implementation
│   ├── docker.rs           # Docker Desktop implementation
│   ├── podman.rs           # Podman implementation
│   └── cloudflare.rs       # Cloudflare Containers + Artifacts
├── identity/
│   ├── mod.rs              # RepoIdentity enum, identity resolution
│   └── radicle.rs          # Radicle URN handling, rad CLI integration
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
│   ├── types.ts            # RuntimeInfo, ContainerSession, ContentRef
│   └── api.ts              # Tauri command wrappers
├── kernels/
│   ├── types.ts            # KernelSession, ExecuteResult
│   └── api.ts              # Tauri command wrappers
└── terminal/
    ├── types.ts            # TerminalSession, TerminalEvent
    └── api.ts              # Tauri command wrappers
```

---

## Part 9 — Security Comparison

| Execution environment | Process isolation | Filesystem isolation | Network isolation | Resource limits | Content provenance | Escape risk |
|---|---|---|---|---|---|---|
| JupyterLite JS kernel | None (same page) | None (same IndexedDB) | CORS/CSP only | None | Origin URL | Browser engine bug |
| JupyterLite Pyodide | WASM linear memory | None (same IndexedDB) | CORS/CSP only | None | Origin URL | WASM engine bug |
| AI SDK Chat kernel | None (same page) | None (same IndexedDB) | CORS/CSP only | None | Origin URL | Browser engine bug |
| Native kernel (Tier 2) | OS process boundary | User-level access | Full network | ulimit | Local workspace | Process escape |
| Apple Container | VM boundary | Image rootfs + mounts | Configurable | cgroup-like | Repo + commit SHA | VM escape |
| Docker | Linux namespace + cgroup | Image rootfs + mounts | Configurable | cgroup | Repo + commit SHA | Container escape |
| Podman (rootless) | User namespace + cgroup | Image rootfs + mounts | Configurable | cgroup | Repo + commit SHA | Container escape (reduced) |
| Cloudflare Container | Remote VM + network | ArtifactFS (git content) | Cloudflare network | Cloudflare-managed | Artifact repo + commit | Remote escape |
| Cloudflare Worker | V8 isolate | ArtifactFS binding | Fetch API only | CPU time + memory | Artifact repo + commit | V8 isolate escape |

The "Content provenance" column reflects how we know what code/data is in the environment. Git-based content refs with commit SHAs provide the strongest provenance: content-addressed, signed (with Radicle), and reproducible.

---

## Part 10 — Implementation Priorities

### Next Milestone

1. **Fix AI SDK Chat kernel** — Graceful fallback when local providers are unavailable in WKWebView. Update CSP to allow cloud provider API domains and Hugging Face CDN. This is a JupyterLite-side fix + a `tauri.conf.json` CSP update.

2. **Container runtime trait + Docker/Podman** — Most users already have Docker. Implement the trait, Docker runtime, and Podman runtime. Add terminal support. Content is mounted from the existing workspace (local git clone).

3. **ContentRef model** — Implement the `ContentRef` / `RepoIdentity` types and the resolution logic that maps repo + commit to local paths for bind-mounting.

### Subsequent Milestones

4. **Apple Containers** — Add native macOS container support. Same `ContainerRuntime` trait, same content model.

5. **Container-backed kernels** — Use the container runtime to run real Jupyter kernels (`ipykernel`, `tslab`, etc.) inside containers, wiring them to JupyterLite's kernel protocol via a bridge kernel extension.

6. **Radicle identity** — Add `RadicleRepoProvider`, support Radicle URNs in `RepoIdentity`, enable cryptographic provenance for content refs and agent identity.

7. **Cloudflare Containers + Artifacts** — Add remote container execution with git-native content via Artifacts. Enables agent workflows: create per-agent Artifact repos, run in isolated containers, diff/merge results.

8. **Additional in-browser kernels** — Add TypeScript transpilation, webR, or other WASM kernels as JupyterLite extensions (no Tauri host changes needed).

9. **WebGPU support** — Track Apple's WKWebView WebGPU availability. When enabled, the WebLLM kernel will work in the desktop app.
