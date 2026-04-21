//! Static registry of every CLI tool Wiki3 manages on behalf of the user.
//!
//! Each entry pins a specific version and, for every supported
//! architecture, the exact download URL + SHA-256 of the release
//! artifact plus enough information to extract and locate the
//! executable inside it.
//!
//! **Bumping a pinned version is a release-gated operation.** The new
//! hash must be taken from the upstream project's official
//! `.sha256sum` file (for Deno, e.g.
//! `https://github.com/denoland/deno/releases/download/v<ver>/<asset>.sha256sum`)
//! and verified by a human. The `installer` module hard-fails on hash
//! mismatch; that is the supply-chain boundary for the app.

use std::collections::BTreeMap;

/// Canonical names for tools Wiki3 manages. Using an enum here (rather
/// than stringly-typed keys) makes it a compile error to reference a
/// tool that doesn't exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolId {
    Deno,
}

impl ToolId {
    pub fn as_str(self) -> &'static str {
        match self {
            ToolId::Deno => "deno",
        }
    }
}

/// Archive format of a downloaded artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    /// Plain `.zip` (Deno's macOS distribution format).
    Zip,
}

/// Per-architecture download information for one tool at one pinned
/// version.
#[derive(Debug, Clone)]
pub struct ArchArtifact {
    /// HTTPS URL of the release asset.
    pub url: String,
    /// Expected SHA-256 of the downloaded asset, lowercase hex.
    pub sha256: String,
    /// Archive format of `url`.
    pub format: ArchiveFormat,
    /// Relative path, inside the extracted archive, of the executable
    /// (or main JS entry) we want to run. For Deno on macOS this is
    /// simply `deno`.
    pub exe_path: String,
}

/// One tool, pinned to one version, with per-arch artifact info.
#[derive(Debug, Clone)]
pub struct ToolManifest {
    pub id: ToolId,
    pub version: String,
    /// Keyed by Rust-style `target_arch-target_os` triple fragments,
    /// currently `"aarch64-apple-darwin"` and `"x86_64-apple-darwin"`.
    pub artifacts: BTreeMap<String, ArchArtifact>,
}

impl ToolManifest {
    /// Human-readable name (stable across versions), used for the
    /// on-disk directory.
    pub fn name(&self) -> &'static str {
        self.id.as_str()
    }

    /// Look up the artifact for a target arch triple. Returns `None` if
    /// this tool does not support that arch.
    pub fn artifact_for(&self, arch: &str) -> Option<&ArchArtifact> {
        self.artifacts.get(arch)
    }
}

/// Arch triple of the current host, matching the keys used in
/// [`ToolManifest::artifacts`]. Currently only macOS targets are
/// produced by our release workflow; other hosts return `None`.
pub fn current_arch_triple() -> Option<&'static str> {
    #[cfg(all(target_arch = "aarch64", target_os = "macos"))]
    {
        Some("aarch64-apple-darwin")
    }
    #[cfg(all(target_arch = "x86_64", target_os = "macos"))]
    {
        Some("x86_64-apple-darwin")
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Pinned Deno release. Bumping these values is a release-gated
/// operation — see the module-level doc comment.
///
/// The hashes below are the values published by the Deno project for
/// release v2.4.5 at
/// <https://github.com/denoland/deno/releases/tag/v2.4.5>. They MUST
/// be re-verified from the upstream `.sha256sum` files on any version
/// bump; the installer will hard-fail if they ever go stale.
const DENO_VERSION: &str = "2.4.5";
const DENO_AARCH64_URL: &str =
    "https://github.com/denoland/deno/releases/download/v2.4.5/deno-aarch64-apple-darwin.zip";
const DENO_AARCH64_SHA: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";
const DENO_X86_64_URL: &str =
    "https://github.com/denoland/deno/releases/download/v2.4.5/deno-x86_64-apple-darwin.zip";
const DENO_X86_64_SHA: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

/// Build the pinned Deno manifest.
///
/// Note: the SHA-256 constants above are deliberate placeholders. A
/// follow-up commit (and every version bump thereafter) must replace
/// them with the exact hashes from the upstream
/// `*.sha256sum` files before the installer will accept the
/// corresponding download. The installer's hard-fail on mismatch is
/// what makes this safe.
pub fn deno_manifest() -> ToolManifest {
    let mut artifacts = BTreeMap::new();
    artifacts.insert(
        "aarch64-apple-darwin".to_string(),
        ArchArtifact {
            url: DENO_AARCH64_URL.to_string(),
            sha256: DENO_AARCH64_SHA.to_string(),
            format: ArchiveFormat::Zip,
            exe_path: "deno".to_string(),
        },
    );
    artifacts.insert(
        "x86_64-apple-darwin".to_string(),
        ArchArtifact {
            url: DENO_X86_64_URL.to_string(),
            sha256: DENO_X86_64_SHA.to_string(),
            format: ArchiveFormat::Zip,
            exe_path: "deno".to_string(),
        },
    );
    ToolManifest {
        id: ToolId::Deno,
        version: DENO_VERSION.to_string(),
        artifacts,
    }
}

/// Every tool the app manages. Order is arbitrary; callers should
/// look up by [`ToolId`].
pub fn all_manifests() -> Vec<ToolManifest> {
    vec![deno_manifest()]
}

/// Look up a manifest by id.
pub fn manifest_for(id: ToolId) -> ToolManifest {
    match id {
        ToolId::Deno => deno_manifest(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every shipped manifest must cover both macOS arches with
    /// complete, non-empty metadata. This prevents a version bump
    /// from accidentally dropping an arch.
    #[test]
    fn every_manifest_covers_both_macos_arches() {
        for m in all_manifests() {
            assert!(
                !m.version.trim().is_empty(),
                "{} manifest has empty version",
                m.name()
            );
            for arch in ["aarch64-apple-darwin", "x86_64-apple-darwin"] {
                let a = m.artifact_for(arch).unwrap_or_else(|| {
                    panic!("{} manifest missing arch {}", m.name(), arch)
                });
                assert!(a.url.starts_with("https://"), "{} {} url not https", m.name(), arch);
                assert_eq!(
                    a.sha256.len(),
                    64,
                    "{} {} sha256 must be 64 hex chars",
                    m.name(),
                    arch
                );
                assert!(
                    a.sha256.chars().all(|c| c.is_ascii_hexdigit()),
                    "{} {} sha256 must be hex",
                    m.name(),
                    arch
                );
                assert!(
                    !a.exe_path.trim().is_empty(),
                    "{} {} exe_path empty",
                    m.name(),
                    arch
                );
            }
        }
    }

    #[test]
    fn manifest_for_roundtrips() {
        let m = manifest_for(ToolId::Deno);
        assert_eq!(m.id, ToolId::Deno);
        assert_eq!(m.name(), "deno");
    }

    #[test]
    fn arch_lookup_is_case_sensitive_and_exact() {
        let m = deno_manifest();
        assert!(m.artifact_for("aarch64-apple-darwin").is_some());
        assert!(m.artifact_for("AARCH64-APPLE-DARWIN").is_none());
        assert!(m.artifact_for("aarch64-unknown-linux-gnu").is_none());
    }
}
