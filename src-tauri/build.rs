//! Build script for the Wiki3 desktop app.
//!
//! In addition to `tauri_build::build()`, this script **bundles the
//! pinned Deno binary into the app** so the installed app ships with
//! its own Deno and the user never has to install one. The binary is
//! downloaded from the pinned URL in `src/tools/registry.rs`,
//! verified against the pinned SHA-256, unzipped, and staged at
//! `src-tauri/resources/deno-<target-triple>`. Tauri's bundler then
//! copies that file into `<Wiki3.app>/Contents/Resources/`, where the
//! runtime code resolves it via `BaseDirectory::Resource`.
//!
//! Gating:
//! * Runs only when `CARGO_CFG_TARGET_OS == "macos"`, since that is
//!   the only platform the Wiki3 app ships for. On Linux / Windows
//!   (including CI tests) this build script is a no-op, so
//!   `cargo check` / `cargo test` continue to work without network.
//! * Skipped entirely when `WIKI3_SKIP_BUNDLED_DENO=1` is set. Useful
//!   for offline dev builds where you want to stage the binary
//!   manually into `src-tauri/resources/`.
//! * If the target file already exists, the script is a cache-hit
//!   no-op. Delete the file to force a re-fetch.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

// Pinned Deno release. MUST be kept in sync with the values in
// src/tools/registry.rs (a test enforces this, see below).
const DENO_VERSION: &str = "2.4.5";
const DENO_AARCH64_URL: &str =
    "https://github.com/denoland/deno/releases/download/v2.4.5/deno-aarch64-apple-darwin.zip";
const DENO_AARCH64_SHA: &str =
    "d21374dc6aa02b493467ec2f6d865cab95cffebb89aab242dc1acf95274681ee";
const DENO_X86_64_URL: &str =
    "https://github.com/denoland/deno/releases/download/v2.4.5/deno-x86_64-apple-darwin.zip";
const DENO_X86_64_SHA: &str =
    "cd46e3d5c06fbd21ef12742def67cc12ef83a2f99d44e00419615f99de056574";

fn main() {
    // Always tell cargo about the knobs that could invalidate the
    // cached artifact, regardless of target.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=WIKI3_SKIP_BUNDLED_DENO");

    if let Err(e) = stage_bundled_deno() {
        // build.rs must not swallow a real failure — if we claim the
        // app ships with Deno, a broken build should not succeed.
        panic!("Failed to bundle Deno: {e}");
    }

    tauri_build::build();
}

fn stage_bundled_deno() -> Result<(), String> {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").map_err(|e| e.to_string())?);
    let resources = manifest_dir.join("resources");
    fs::create_dir_all(&resources).map_err(|e| format!("mkdir resources: {e}"))?;

    // Tauri's bundler insists the resources glob match at least one
    // file. On non-macOS hosts (CI / Linux dev) we don't stage a real
    // Deno; drop a placeholder instead so `cargo check`/`cargo test`
    // succeed. The placeholder is never shipped (macOS is the only
    // release target).
    let placeholder = resources.join("deno-placeholder");
    if !placeholder.exists() {
        fs::write(&placeholder, b"placeholder\n")
            .map_err(|e| format!("write placeholder: {e}"))?;
    }

    if target_os != "macos" {
        println!(
            "cargo:warning=Wiki3 build.rs: target_os={target_os}, skipping Deno bundling"
        );
        return Ok(());
    }
    if env::var("WIKI3_SKIP_BUNDLED_DENO").is_ok() {
        println!(
            "cargo:warning=WIKI3_SKIP_BUNDLED_DENO set; skipping Deno bundling. \
             The resulting app will not run devcontainer builds."
        );
        return Ok(());
    }

    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let (triple, url, sha) = match target_arch.as_str() {
        "aarch64" => ("aarch64-apple-darwin", DENO_AARCH64_URL, DENO_AARCH64_SHA),
        "x86_64" => ("x86_64-apple-darwin", DENO_X86_64_URL, DENO_X86_64_SHA),
        other => return Err(format!("unsupported target arch: {other}")),
    };

    let staged = resources.join(format!("deno-{triple}"));

    if staged.is_file() {
        // Verify the cached copy still matches the pinned SHA. Guards
        // against a partially-written or tampered-with file surviving
        // between builds.
        let got = sha256_file(&staged)?;
        if got.eq_ignore_ascii_case(sha) {
            return Ok(());
        }
        println!(
            "cargo:warning=staged Deno at {:?} has wrong hash (got {}, expected {}); re-fetching",
            staged, got, sha
        );
        fs::remove_file(&staged).map_err(|e| format!("remove stale: {e}"))?;
    }

    let out_dir =
        PathBuf::from(env::var("OUT_DIR").map_err(|e| e.to_string())?);
    let zip_path = out_dir.join(format!("deno-{triple}-{DENO_VERSION}.zip"));

    // curl is universally available on macOS; avoids adding a
    // reqwest/blocking build-dep that would balloon compile time.
    println!(
        "cargo:warning=Downloading bundled Deno {DENO_VERSION} for {triple}…"
    );
    let status = Command::new("curl")
        .args(["-fsSL", "--retry", "3", "-o"])
        .arg(&zip_path)
        .arg(url)
        .status()
        .map_err(|e| format!("spawn curl: {e}"))?;
    if !status.success() {
        return Err(format!("curl failed ({status}) for {url}"));
    }

    let got = sha256_file(&zip_path)?;
    if !got.eq_ignore_ascii_case(sha) {
        // Supply-chain boundary: a hash mismatch must fail the build.
        fs::remove_file(&zip_path).ok();
        return Err(format!(
            "hash mismatch for {url}: expected {sha}, got {got}"
        ));
    }

    let extract_dir = out_dir.join(format!("deno-{triple}-{DENO_VERSION}"));
    let _ = fs::remove_dir_all(&extract_dir);
    fs::create_dir_all(&extract_dir).map_err(|e| format!("mkdir extract: {e}"))?;

    let status = Command::new("unzip")
        .arg("-o")
        .arg(&zip_path)
        .arg("-d")
        .arg(&extract_dir)
        .status()
        .map_err(|e| format!("spawn unzip: {e}"))?;
    if !status.success() {
        return Err(format!("unzip failed ({status})"));
    }

    let src = extract_dir.join("deno");
    if !src.is_file() {
        return Err(format!("archive did not contain a `deno` file at {src:?}"));
    }
    fs::copy(&src, &staged).map_err(|e| format!("copy {src:?} -> {staged:?}: {e}"))?;

    // The extracted binary is already executable, but copy() does not
    // always preserve the bit — set it explicitly.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&staged)
            .map_err(|e| format!("stat staged: {e}"))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&staged, perms)
            .map_err(|e| format!("chmod staged: {e}"))?;
    }

    println!("cargo:rerun-if-changed={}", staged.display());
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    // Prefer the bundled `shasum` (universal on macOS + Linux) to
    // avoid a build-dep. Fall back to `sha256sum` if absent.
    for (cmd, args) in [
        ("shasum", &["-a", "256"][..]),
        ("sha256sum", &[][..]),
    ] {
        let out = Command::new(cmd).args(args).arg(path).output();
        match out {
            Ok(o) if o.status.success() => {
                let s = String::from_utf8_lossy(&o.stdout);
                if let Some(hex) = s.split_whitespace().next() {
                    return Ok(hex.to_string());
                }
                return Err(format!("empty hash output from {cmd}"));
            }
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr);
                return Err(format!("{cmd} failed: {err}"));
            }
            Err(_) => continue,
        }
    }
    // Last-resort pure-Rust fallback so build succeeds on unusual
    // hosts. Read in chunks to avoid loading huge archives into RAM.
    let mut file = fs::File::open(path).map_err(|e| format!("open {path:?}: {e}"))?;
    let mut hasher = Sha256Hasher::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| format!("read: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize_hex())
}

// Tiny local SHA-256 so build.rs never pulls in a new crate as a
// build-dep. Only used on the cold path where neither `shasum` nor
// `sha256sum` is available; exercised by the unit test below.
struct Sha256Hasher {
    state: [u32; 8],
    buf: [u8; 64],
    buf_len: usize,
    bits: u64,
}

impl Sha256Hasher {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c,
                0x1f83d9ab, 0x5be0cd19,
            ],
            buf: [0; 64],
            buf_len: 0,
            bits: 0,
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.bits = self.bits.wrapping_add((data.len() as u64) * 8);
        if self.buf_len > 0 {
            let need = 64 - self.buf_len;
            let take = data.len().min(need);
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len == 64 {
                let block = self.buf;
                self.compress(&block);
                self.buf_len = 0;
            }
        }
        while data.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&data[..64]);
            self.compress(&block);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.buf[..data.len()].copy_from_slice(data);
            self.buf_len = data.len();
        }
    }

    fn finalize_hex(mut self) -> String {
        let bits = self.bits;
        // pad
        let mut tail = [0u8; 128];
        tail[0] = 0x80;
        let pad_len = if self.buf_len < 56 { 56 - self.buf_len } else { 120 - self.buf_len };
        let bits_be = bits.to_be_bytes();
        tail[pad_len..pad_len + 8].copy_from_slice(&bits_be);
        let total = pad_len + 8;
        self.update(&tail[..total]);
        let mut out = String::with_capacity(64);
        for w in self.state.iter() {
            for b in w.to_be_bytes().iter() {
                let _ = write!(&mut out, "{:02x}", b);
            }
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1,
            0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
            0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
            0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
            0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
            0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
            0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
            0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
            0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
            0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
        ];
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7)
                ^ w[i - 15].rotate_right(18)
                ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17)
                ^ w[i - 2].rotate_right(19)
                ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }
}
