//! Hash-verified installer for managed CLI tools.
//!
//! `ensure` is idempotent: if `<tools_dir>/<name>/<version>/` already
//! exists, it's treated as a valid cached install (it could only have
//! been created by a prior successful atomic rename after hash
//! verification). Otherwise `ensure` downloads, verifies, extracts,
//! and atomically moves the unpacked tree into place.
//!
//! The hash check is the **supply-chain boundary**: any mismatch is a
//! hard failure, never silently retried or downgraded.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::registry::{ArchiveFormat, ToolManifest};
use super::{Result, ToolsError};

/// Progress events reported during `ensure`. Designed to be trivially
/// forwardable to a Tauri event stream by the caller without coupling
/// this module to Tauri.
#[derive(Debug, Clone)]
pub enum InstallProgress {
    Starting { name: String, version: String },
    CacheHit { name: String, version: String },
    Downloading {
        name: String,
        downloaded: u64,
        total: Option<u64>,
    },
    Verifying { name: String },
    Extracting { name: String },
    Done { name: String, version: String },
}

/// Ensure the tool described by `manifest` is installed under
/// `tools_dir`, for the given target arch triple. Returns the absolute
/// path to the executable (or main entry file) inside the install.
///
/// If already cached, returns immediately after emitting `CacheHit`.
pub async fn ensure(
    tools_dir: &Path,
    manifest: &ToolManifest,
    arch: &str,
    mut progress: impl FnMut(InstallProgress),
) -> Result<PathBuf> {
    let artifact = manifest
        .artifact_for(arch)
        .ok_or_else(|| ToolsError::UnsupportedArch {
            name: manifest.name().to_string(),
            arch: arch.to_string(),
        })?;

    progress(InstallProgress::Starting {
        name: manifest.name().to_string(),
        version: manifest.version.clone(),
    });

    let install_dir = tools_dir
        .join(manifest.name())
        .join(&manifest.version);
    let exe = install_dir.join(&artifact.exe_path);

    // If the pinned version directory already exists, we trust it:
    // it could only have been put there by a prior successful atomic
    // rename after hash verification. A partially-extracted install is
    // impossible because extraction happens in a sibling temp dir.
    if install_dir.exists() {
        progress(InstallProgress::CacheHit {
            name: manifest.name().to_string(),
            version: manifest.version.clone(),
        });
        return Ok(exe);
    }

    // Parent dirs.
    fs::create_dir_all(tools_dir.join(manifest.name()))?;

    // Download to a temp file under tools_dir (same filesystem ->
    // rename is atomic later).
    let staging_root = tools_dir.join(manifest.name()).join(".staging");
    let _ = fs::remove_dir_all(&staging_root);
    fs::create_dir_all(&staging_root)?;

    let archive_path = staging_root.join("download");
    let actual_hash = download_with_hash(
        &artifact.url,
        &archive_path,
        manifest.name(),
        &mut progress,
    )
    .await?;

    progress(InstallProgress::Verifying {
        name: manifest.name().to_string(),
    });
    if !hash_equals_ci(&actual_hash, &artifact.sha256) {
        // Clean up the bad download before bailing.
        let _ = fs::remove_dir_all(&staging_root);
        return Err(ToolsError::HashMismatch {
            name: manifest.name().to_string(),
            version: manifest.version.clone(),
            arch: arch.to_string(),
            expected: artifact.sha256.clone(),
            actual: actual_hash,
        });
    }

    // Extract into a temp subdir, then atomically rename into place.
    progress(InstallProgress::Extracting {
        name: manifest.name().to_string(),
    });
    let extract_dir = staging_root.join("extract");
    fs::create_dir_all(&extract_dir)?;
    match artifact.format {
        ArchiveFormat::Zip => extract_zip(&archive_path, &extract_dir)?,
    }

    // Atomic rename into the version directory. If the rename races
    // with another process that finished first, we yield to them.
    match fs::rename(&extract_dir, &install_dir) {
        Ok(()) => {}
        Err(e) => {
            if install_dir.exists() {
                // Someone else won the race; treat as success.
            } else {
                let _ = fs::remove_dir_all(&staging_root);
                return Err(ToolsError::Io(e));
            }
        }
    }

    // Best-effort staging cleanup (archive file, leftover dirs).
    let _ = fs::remove_dir_all(&staging_root);

    // On Unix, restore execute permission on the entry file — it is
    // preserved by zip on macOS, but be defensive for downloads where
    // it isn't.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(md) = fs::metadata(&exe) {
            let mut perms = md.permissions();
            let mode = perms.mode() | 0o755;
            perms.set_mode(mode);
            let _ = fs::set_permissions(&exe, perms);
        }
    }

    progress(InstallProgress::Done {
        name: manifest.name().to_string(),
        version: manifest.version.clone(),
    });
    Ok(exe)
}

/// Stream a URL to `dest`, computing SHA-256 on the fly. Returns the
/// lowercase-hex hash of the downloaded bytes.
async fn download_with_hash(
    url: &str,
    dest: &Path,
    tool_name: &str,
    progress: &mut impl FnMut(InstallProgress),
) -> Result<String> {
    let resp = reqwest::get(url)
        .await
        .map_err(|e| ToolsError::Download {
            url: url.to_string(),
            source: e,
        })?
        .error_for_status()
        .map_err(|e| ToolsError::Download {
            url: url.to_string(),
            source: e,
        })?;

    let total = resp.content_length();
    let mut stream = resp.bytes_stream();
    let mut file = fs::File::create(dest)?;
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;

    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| ToolsError::Download {
            url: url.to_string(),
            source: e,
        })?;
        file.write_all(&chunk)?;
        hasher.update(&chunk);
        downloaded += chunk.len() as u64;
        progress(InstallProgress::Downloading {
            name: tool_name.to_string(),
            downloaded,
            total,
        });
    }
    file.flush()?;
    drop(file);

    let digest = hasher.finalize();
    Ok(hex_encode(&digest))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn hash_equals_ci(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .map(|c| c.to_ascii_lowercase())
            .eq(b.bytes().map(|c| c.to_ascii_lowercase()))
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<()> {
    let file = fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| ToolsError::Extract(format!("open zip: {e}")))?;

    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| ToolsError::Extract(format!("zip entry {i}: {e}")))?;

        // `zip` 2.3+ canonicalizes `enclosed_name()` to reject
        // path-traversal entries (the CVE-2025-29787 fix); if the
        // entry's name is unsafe, skip it.
        let rel = match entry.enclosed_name() {
            Some(p) => p.to_path_buf(),
            None => continue,
        };
        let out_path = dest.join(rel);

        if entry.is_dir() {
            fs::create_dir_all(&out_path)?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out = fs::File::create(&out_path)?;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = entry.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = entry.unix_mode() {
                let _ = fs::set_permissions(&out_path, fs::Permissions::from_mode(mode));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::registry::{ArchArtifact, ArchiveFormat, ToolId, ToolManifest};
    use std::collections::BTreeMap;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    /// A tiny HTTP fixture serving a fixed byte payload at `GET /file`.
    /// Returned values: bound URL and a counter of requests received.
    fn spawn_fixture(body: Vec<u8>) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let url = format!("http://{addr}/file");
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_clone = hits.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = match stream {
                    Ok(s) => s,
                    Err(_) => return,
                };
                hits_clone.fetch_add(1, Ordering::SeqCst);

                // Read and discard the request head.
                {
                    let mut reader = BufReader::new(&stream);
                    let mut line = String::new();
                    loop {
                        line.clear();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 {
                            break;
                        }
                        if line == "\r\n" || line == "\n" {
                            break;
                        }
                    }
                }

                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/zip\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        });
        (url, hits)
    }

    /// Build a minimal zip containing a single file `deno` with given
    /// bytes. Uses the `zip` crate so we don't hand-roll the format.
    fn make_zip(exe_bytes: &[u8]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::<u8>::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let opts: zip::write::SimpleFileOptions =
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored)
                    .unix_permissions(0o755);
            w.start_file("deno", opts).unwrap();
            w.write_all(exe_bytes).unwrap();
            w.finish().unwrap();
        }
        buf.into_inner()
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(bytes);
        hex_encode(&h.finalize())
    }

    fn manifest_with(url: &str, sha: &str) -> ToolManifest {
        let mut artifacts = BTreeMap::new();
        artifacts.insert(
            "test-arch".to_string(),
            ArchArtifact {
                url: url.to_string(),
                sha256: sha.to_string(),
                format: ArchiveFormat::Zip,
                exe_path: "deno".to_string(),
            },
        );
        ToolManifest {
            id: ToolId::Deno,
            version: "9.9.9".to_string(),
            artifacts,
        }
    }

    #[tokio::test]
    async fn happy_path_installs_and_is_idempotent() {
        let zip_bytes = make_zip(b"#!/usr/bin/env bash\necho hi\n");
        let hash = sha256_hex(&zip_bytes);
        let (url, hits) = spawn_fixture(zip_bytes);
        let tmp = tempfile::tempdir().unwrap();
        let m = manifest_with(&url, &hash);

        let exe = ensure(tmp.path(), &m, "test-arch", |_| {})
            .await
            .expect("install");
        assert!(exe.is_file(), "exe should exist: {exe:?}");
        assert_eq!(exe.file_name().unwrap(), "deno");
        assert_eq!(hits.load(Ordering::SeqCst), 1);

        // Second call is a no-op cache hit — no new HTTP request.
        let exe2 = ensure(tmp.path(), &m, "test-arch", |_| {})
            .await
            .expect("cache hit");
        assert_eq!(exe, exe2);
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "cache hit must not refetch"
        );
    }

    #[tokio::test]
    async fn hash_mismatch_is_hard_fail_and_does_not_install() {
        let zip_bytes = make_zip(b"real contents");
        let (url, _) = spawn_fixture(zip_bytes);
        let wrong_hash = "f".repeat(64);
        let tmp = tempfile::tempdir().unwrap();
        let m = manifest_with(&url, &wrong_hash);

        let err = ensure(tmp.path(), &m, "test-arch", |_| {})
            .await
            .expect_err("should reject");
        assert!(matches!(err, ToolsError::HashMismatch { .. }));

        // Install directory must not exist.
        let install_dir = tmp.path().join("deno").join("9.9.9");
        assert!(!install_dir.exists(), "bad install must not be materialized");
        // Staging must have been cleaned up too.
        let staging = tmp.path().join("deno").join(".staging");
        assert!(!staging.exists(), "staging must be cleaned up");
    }

    #[tokio::test]
    async fn unsupported_arch_errors_before_network() {
        // No server: any HTTP would hang, proving we never tried.
        let m = manifest_with("http://127.0.0.1:1/unused", &"0".repeat(64));
        let tmp = tempfile::tempdir().unwrap();
        let err = ensure(tmp.path(), &m, "no-such-arch", |_| {})
            .await
            .expect_err("should reject arch");
        assert!(matches!(err, ToolsError::UnsupportedArch { .. }));
    }

    #[test]
    fn hex_encode_is_lowercase() {
        assert_eq!(hex_encode(&[0x00, 0xab, 0xff]), "00abff");
    }

    #[test]
    fn hash_equals_is_case_insensitive() {
        assert!(hash_equals_ci("abcd", "ABCD"));
        assert!(hash_equals_ci("00ff", "00FF"));
        assert!(!hash_equals_ci("abcd", "abce"));
        assert!(!hash_equals_ci("abcd", "abc"));
    }
}
