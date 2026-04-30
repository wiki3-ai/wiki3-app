// Loader for the prebuilt devcontainer engine bundle.
//
// The bundle is the same artefact shipped by `devcontainer-cli`'s
// frontend (built by Deno + esbuild from
// `frontend/src/devcontainer-engine/index.ts` over there). It is
// dropped into `wiki3-app/src/public/devcontainer-engine.js` and
// fetched at runtime so the WebView gets to use the upstream
// devcontainers/cli spec slice unmodified.
//
// The bundle's `tauriFileHost()` calls Tauri commands `fs_is_file`,
// `fs_read_file`, `fs_write_file`, `fs_read_dir`, `fs_mkdirp`. Its
// `loadAndSubmitDevContainerConfig()` additionally calls
// `submit_parsed_devcontainer`. All of those are implemented in
// `src-tauri/src/commands_devcontainer.rs`.

export interface ParsedDevContainer {
  name?: string;
  image?: string;
  build?: { dockerfile?: string; context?: string; args?: Record<string, string>; target?: string };
  configFilePath?: string;
  workspaceFolder?: string;
  workspaceMount?: string;
  mounts: string[];
  forwardPorts: number[];
  runArgs: string[];
  remoteUser?: string;
  containerEnv: Record<string, string>;
  remoteEnv: Record<string, string | null>;
  postCreateCommand?: string | string[];
  postStartCommand?: string | string[];
  postAttachCommand?: string | string[];
}

export interface LoadConfigResult {
  configFilePath: string;
  parsed: ParsedDevContainer;
  raw: unknown;
}

interface FileHost {
  platform: string;
  path: unknown;
  isFile: (p: string) => Promise<boolean>;
  readFile: (p: string) => Promise<Uint8Array>;
  writeFile: (p: string, content: Uint8Array) => Promise<void>;
  readDir: (p: string) => Promise<string[]>;
  mkdirp: (p: string) => Promise<void>;
  toCommonURI: () => Promise<undefined>;
}

interface EngineModule {
  tauriFileHost: () => FileHost;
  loadDevContainerConfig: (
    fileHost: FileHost,
    workspacePath: string,
    env?: Record<string, string | undefined>,
  ) => Promise<LoadConfigResult | undefined>;
  loadAndSubmitDevContainerConfig: (
    fileHost: FileHost,
    workspaceId: string,
    workspacePath: string,
    env?: Record<string, string | undefined>,
  ) => Promise<LoadConfigResult | undefined>;
}

let enginePromise: Promise<EngineModule> | undefined;

function loadEngine(): Promise<EngineModule> {
  if (!enginePromise) {
    enginePromise = (async () => {
      const res = await fetch('/devcontainer-engine.js');
      if (!res.ok) {
        throw new Error(`Failed to fetch engine bundle: ${res.status} ${res.statusText}`);
      }
      const code = await res.text();
      const url = URL.createObjectURL(new Blob([code], { type: 'text/javascript' }));
      try {
        return (await import(/* @vite-ignore */ url)) as EngineModule;
      } finally {
        URL.revokeObjectURL(url);
      }
    })();
  }
  return enginePromise;
}

/**
 * Read `.devcontainer/devcontainer.json` for `wikiId` (located at
 * `wikiPath`), parse it through the upstream spec slice, and submit
 * the result to the Rust orchestrator. Subsequent
 * `wiki_container_ctl_*` commands then operate against it.
 *
 * Returns the parsed result, or `undefined` if no config file exists.
 */
export async function loadAndSubmitDevcontainer(
  wikiId: string,
  wikiPath: string,
): Promise<LoadConfigResult | undefined> {
  const eng = await loadEngine();
  return eng.loadAndSubmitDevContainerConfig(eng.tauriFileHost(), wikiId, wikiPath);
}
