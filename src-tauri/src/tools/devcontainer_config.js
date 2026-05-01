// devcontainer_config.js
// Embedded in the wiki3-app binary via include_str!.
//
// Exports one global function:
//   resolveConfig(jsonStr: string) -> string (JSON)
//
// Parses a devcontainer.json string, normalises it according to the
// devcontainer spec (https://containers.dev/implementors/json_reference/),
// validates that a runtime source (image or build) is present, and
// returns a JSON string of the normalised config.  Throws on bad input.
//
// All I/O is handled by the Rust caller; this module is pure logic so
// it can run in QuickJS without any host APIs.

// Strip JSONC features (// and /* */ comments, trailing commas) so
// JSON.parse accepts the input. devcontainer.json is officially JSONC
// per the spec, and VS Code / the upstream CLI both tolerate these.
// String contents are preserved verbatim — we walk character-by-
// character and only treat comment/comma syntax as such when outside
// a JSON string literal.
function stripJsonc(src) {
  var out = '';
  var i = 0;
  var n = src.length;
  while (i < n) {
    var c = src.charAt(i);
    // Inside a string: copy through to the closing quote, honouring
    // backslash escapes so an escaped quote doesn't end the string.
    if (c === '"') {
      out += c;
      i++;
      while (i < n) {
        var d = src.charAt(i);
        out += d;
        i++;
        if (d === '\\' && i < n) {
          out += src.charAt(i);
          i++;
        } else if (d === '"') {
          break;
        }
      }
      continue;
    }
    // Line comment: skip until end-of-line (preserve the newline so
    // line numbers in JSON.parse errors stay meaningful).
    if (c === '/' && i + 1 < n && src.charAt(i + 1) === '/') {
      i += 2;
      while (i < n && src.charAt(i) !== '\n') i++;
      continue;
    }
    // Block comment: skip until */ (preserve embedded newlines).
    if (c === '/' && i + 1 < n && src.charAt(i + 1) === '*') {
      i += 2;
      while (i < n) {
        if (src.charAt(i) === '*' && i + 1 < n && src.charAt(i + 1) === '/') {
          i += 2;
          break;
        }
        if (src.charAt(i) === '\n') out += '\n';
        i++;
      }
      continue;
    }
    out += c;
    i++;
  }
  // Trailing commas before } or ]: drop them. Safe because we already
  // stripped strings/comments above (commas inside strings won't match
  // since this regex is run on the stripped output).
  return out.replace(/,(\s*[}\]])/g, '$1');
}

function resolveConfig(jsonStr) {
  var raw;
  try {
    raw = JSON.parse(stripJsonc(jsonStr));
  } catch (e) {
    throw new Error('Invalid JSON in devcontainer.json: ' + e.message);
  }

  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) {
    throw new Error('devcontainer.json must be a JSON object at the root');
  }

  var out = {};

  // --- name ----------------------------------------------------------------
  if (typeof raw.name === 'string' && raw.name.trim() !== '') {
    out.name = raw.name.trim();
  }

  // --- image ---------------------------------------------------------------
  if (typeof raw.image === 'string' && raw.image.trim() !== '') {
    out.image = raw.image.trim();
  }

  // --- build ---------------------------------------------------------------
  // Prefer the structured 'build' object; fall back to the legacy
  // root-level 'dockerFile' / 'dockerfile' key (deprecated but common).
  if (raw.build && typeof raw.build === 'object' && !Array.isArray(raw.build)) {
    var b = {};
    var df = raw.build.dockerfile || raw.build.dockerFile;
    if (typeof df === 'string' && df.trim() !== '') b.dockerfile = df.trim();
    if (typeof raw.build.context === 'string') b.context = raw.build.context;
    if (raw.build.args !== undefined) b.args = raw.build.args;
    if (typeof raw.build.target === 'string') b.target = raw.build.target;
    if (Object.keys(b).length > 0) out.build = b;
  } else {
    var legacyDf = raw.dockerFile || raw.dockerfile;
    if (typeof legacyDf === 'string' && legacyDf.trim() !== '') {
      out.build = { dockerfile: legacyDf.trim() };
    }
  }

  // --- validate: must have image or build ----------------------------------
  if (!out.image && !out.build) {
    throw new Error(
      'devcontainer.json must specify either "image" or "build" (got neither)'
    );
  }

  // --- forwardPorts --------------------------------------------------------
  if (Array.isArray(raw.forwardPorts) && raw.forwardPorts.length > 0) {
    out.forwardPorts = raw.forwardPorts;
  }

  // --- portsAttributes -----------------------------------------------------
  // Pass through verbatim. Keyed by port number as a string ("3000") with
  // values like { label, onAutoForward, protocol, ... }.
  if (
    raw.portsAttributes &&
    typeof raw.portsAttributes === 'object' &&
    !Array.isArray(raw.portsAttributes)
  ) {
    out.portsAttributes = raw.portsAttributes;
  }

  // --- lifecycle commands --------------------------------------------------
  if (raw.postCreateCommand !== undefined) {
    out.postCreateCommand = raw.postCreateCommand;
  }
  if (raw.postStartCommand !== undefined) {
    out.postStartCommand = raw.postStartCommand;
  }
  if (raw.postAttachCommand !== undefined) {
    out.postAttachCommand = raw.postAttachCommand;
  }
  if (raw.initializeCommand !== undefined) {
    out.initializeCommand = raw.initializeCommand;
  }

  // --- user ----------------------------------------------------------------
  if (typeof raw.remoteUser === 'string' && raw.remoteUser.trim() !== '') {
    out.remoteUser = raw.remoteUser.trim();
  }
  if (typeof raw.containerUser === 'string' && raw.containerUser.trim() !== '') {
    out.containerUser = raw.containerUser.trim();
  }

  // --- workspace -----------------------------------------------------------
  if (typeof raw.workspaceFolder === 'string') out.workspaceFolder = raw.workspaceFolder;
  if (typeof raw.workspaceMount === 'string') out.workspaceMount = raw.workspaceMount;

  // --- features ------------------------------------------------------------
  // features is an object keyed by feature OCI ref, values are option objects.
  if (raw.features && typeof raw.features === 'object' && !Array.isArray(raw.features)) {
    out.features = raw.features;
  }

  // --- mounts / runArgs ----------------------------------------------------
  if (Array.isArray(raw.mounts)) out.mounts = raw.mounts;
  if (Array.isArray(raw.runArgs)) out.runArgs = raw.runArgs;

  // --- customizations (e.g. VS Code extensions) ----------------------------
  if (raw.customizations && typeof raw.customizations === 'object') {
    out.customizations = raw.customizations;
  }

  return JSON.stringify(out);
}
