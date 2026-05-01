/**
 * Build / packaging guard tests.
 *
 * These catch regressions that only manifest in a packaged release
 * build — chiefly the CSP-vs-engine-bundle interaction documented in
 * `devcontainers-cli/docs/building-on-macos.md`:
 *
 *   "Without `blob:` in `script-src`, the engine dynamic import
 *    will fail in the packaged release build with a CSP violation."
 *
 * `src/devcontainer-engine.ts` fetches `/devcontainer-engine.js`,
 * wraps it in a `Blob`, and dynamic-imports the resulting `blob:`
 * URL. Tauri's dev override is permissive, so this only blows up in
 * production — no `cargo tauri dev` smoke test would catch it.
 *
 * The unit cost is one fast file-read; the saved cost is a full
 * release-rebuild + manual click-through every time the CSP gets
 * touched.
 */

import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const repoRoot = resolve(__dirname, '..');

describe('tauri.conf.json CSP', () => {
  const conf = JSON.parse(
    readFileSync(resolve(repoRoot, 'src-tauri/tauri.conf.json'), 'utf-8'),
  ) as { app: { security: { csp: string } } };

  // Parse the CSP string into a directive map, mirroring how a browser
  // would. Values are split on whitespace; directive names are
  // lowercased so the test is insensitive to author casing.
  function parseCsp(csp: string): Record<string, string[]> {
    const out: Record<string, string[]> = {};
    for (const part of csp.split(';')) {
      const trimmed = part.trim();
      if (!trimmed) continue;
      const [name, ...rest] = trimmed.split(/\s+/);
      out[name.toLowerCase()] = rest;
    }
    return out;
  }

  const directives = parseCsp(conf.app.security.csp);

  it('declares a script-src directive (otherwise default-src governs scripts)', () => {
    expect(directives['script-src']).toBeDefined();
  });

  it('allows blob: URLs in script-src so the engine bundle dynamic import works', () => {
    // The engine loader does:
    //   const url = URL.createObjectURL(new Blob([code], { type: 'text/javascript' }));
    //   await import(url);
    // which requires `blob:` in script-src. Without it the WebView
    // rejects the import with: TypeError: Importing a module script failed.
    expect(directives['script-src']).toContain('blob:');
  });

  it("keeps 'self' in script-src so the loader's fetch of the bundle is allowed", () => {
    // The bundle is served at /devcontainer-engine.js by Vite/Tauri.
    expect(directives['script-src']).toContain("'self'");
  });
});

describe('engine bundle (src/public/devcontainer-engine.js)', () => {
  const bundlePath = resolve(repoRoot, 'src/public/devcontainer-engine.js');

  it('exists at the path served as /devcontainer-engine.js', () => {
    // If this fails the loader will hit a 404 long before CSP enters
    // the picture; surfacing it explicitly avoids a confusing
    // downstream "Importing a module script failed" diagnosis.
    expect(existsSync(bundlePath)).toBe(true);
  });

  it('exports the symbols the loader expects', () => {
    // `src/devcontainer-engine.ts`'s `EngineModule` interface lists
    // `tauriFileHost`, `loadDevContainerConfig`, and
    // `loadAndSubmitDevContainerConfig`. If the bundle is rebuilt
    // without these (e.g. tree-shaking, renamed exports), the WebView
    // surfaces the failure as
    //   "TypeError: eng.tauriFileHost is not a function"
    // which is what we hit when chasing the CSP issue.
    const code = readFileSync(bundlePath, 'utf-8');
    // Match the esbuild `export { ... }` block at the end of the bundle.
    // We assert each name appears as a bare identifier inside an
    // `export {}` so a coincidental string elsewhere doesn't satisfy us.
    const exportBlocks = [...code.matchAll(/export\s*\{([^}]*)\}/g)].map(
      (m) => m[1],
    );
    const exported = new Set<string>();
    for (const block of exportBlocks) {
      for (const segment of block.split(',')) {
        const name = segment.trim().split(/\s+as\s+/).pop();
        if (name) exported.add(name);
      }
    }
    expect(exported).toContain('tauriFileHost');
    expect(exported).toContain('loadDevContainerConfig');
    expect(exported).toContain('loadAndSubmitDevContainerConfig');
  });
});
