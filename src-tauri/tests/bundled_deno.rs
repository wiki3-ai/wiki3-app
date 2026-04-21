//! Integration tests that exercise the **actual bundled Deno binary**.
//!
//! These tests catch bugs like passing a path to Deno 2.x's
//! `--node-modules-dir` flag (which only accepts `auto|manual|none`)
//! that pure-Rust unit tests cannot surface.
//!
//! If the bundled Deno hasn't been staged (e.g. non-macOS host, or
//! `WIKI3_SKIP_BUNDLED_DENO=1`), the tests skip rather than fail so
//! CI on Linux runners stays green.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use wiki3_app_lib::tools::{bundled_deno_path, runner::DEVCONTAINER_CLI_VERSION};

/// Path to the bundled Deno binary staged by `build.rs` into
/// `src-tauri/resources/`. `None` if not present on this host.
fn staged_deno() -> Option<PathBuf> {
    let resource_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources");
    let path = bundled_deno_path(&resource_dir)?;
    path.is_file().then_some(path)
}

macro_rules! require_deno {
    () => {
        match staged_deno() {
            Some(p) => p,
            None => {
                eprintln!(
                    "skipping: bundled Deno not staged (set WIKI3_SKIP_BUNDLED_DENO=0 and rebuild)"
                );
                return;
            }
        }
    };
}

#[test]
fn bundled_deno_reports_expected_major_version() {
    let deno = require_deno!();
    let out = Command::new(&deno)
        .arg("--version")
        .output()
        .expect("spawn bundled deno");
    assert!(out.status.success(), "deno --version failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    // First line looks like `deno 2.4.5 (...)`; we only assert the
    // major, because the pinned SHA in registry.rs is the real lock.
    let first = stdout.lines().next().unwrap_or("");
    assert!(
        first.starts_with("deno 2."),
        "expected Deno 2.x, got: {first:?}"
    );
}

/// The bug we're guarding against: Deno 2.x's `--node-modules-dir`
/// only accepts `auto|manual|none`. Passing a path gets you
/// `invalid value '…' for '--node-modules-dir[=<MODE>]'`.
///
/// This test runs a trivial inline script under every mode we care
/// about and asserts the process exits successfully.
#[test]
fn node_modules_dir_accepts_none_mode() {
    let deno = require_deno!();
    let tmp = tempfile::tempdir().unwrap();
    let out = Command::new(&deno)
        .arg("eval")
        .arg("--node-modules-dir=none")
        .arg("console.log('ok')")
        .env("DENO_DIR", tmp.path())
        .output()
        .expect("spawn bundled deno");
    assert!(
        out.status.success(),
        "deno eval --node-modules-dir=none failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Regression test for the exact failure reported by the user:
/// passing a filesystem path to `--node-modules-dir` must NOT be
/// silently accepted (it has to be a mode). If Deno ever changes this
/// semantics we want a loud failure so we can revisit `runner.rs`.
#[test]
fn node_modules_dir_rejects_filesystem_path() {
    let deno = require_deno!();
    let tmp = tempfile::tempdir().unwrap();
    let bogus = tmp.path().join("node_modules");
    let out = Command::new(&deno)
        .arg("eval")
        .arg(format!("--node-modules-dir={}", bogus.display()))
        .arg("console.log('ok')")
        .env("DENO_DIR", tmp.path())
        .output()
        .expect("spawn bundled deno");
    assert!(
        !out.status.success(),
        "expected Deno to reject path-valued --node-modules-dir but it succeeded"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("auto") && stderr.contains("manual") && stderr.contains("none"),
        "expected mode-list error from Deno, got: {stderr}"
    );
}

/// End-to-end: fetch `@devcontainers/cli` at the pinned version via
/// `npm:` and run it with `--help`. This exercises the exact flag
/// combination `runner::run_devcontainer` uses, so flag regressions
/// in either Deno or the CLI surface here.
///
/// This test hits the network the first time it runs (populates the
/// Deno cache in `DENO_DIR`) and is therefore somewhat slow. It is
/// gated on `WIKI3_RUN_NETWORK_TESTS=1` so `cargo test` stays offline
/// by default.
#[test]
fn devcontainer_cli_help_runs_under_bundled_deno() {
    if std::env::var("WIKI3_RUN_NETWORK_TESTS").ok().as_deref() != Some("1") {
        eprintln!("skipping: set WIKI3_RUN_NETWORK_TESTS=1 to run this test");
        return;
    }
    let deno = require_deno!();
    let tmp = tempfile::tempdir().unwrap();
    let mut child = Command::new(&deno)
        .arg("run")
        .arg("-A")
        .arg("--node-modules-dir=none")
        .arg(format!(
            "npm:@devcontainers/cli@{}",
            DEVCONTAINER_CLI_VERSION
        ))
        .arg("--help")
        .env("DENO_DIR", tmp.path())
        .spawn()
        .expect("spawn bundled deno");

    // Hard 3-minute cap so a hung test is still bounded.
    let deadline = std::time::Instant::now() + Duration::from_secs(180);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                assert!(
                    status.success(),
                    "devcontainer --help failed under bundled deno: {status:?}"
                );
                return;
            }
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("devcontainer --help timed out after 3 minutes");
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(250)),
            Err(e) => panic!("wait failed: {e}"),
        }
    }
}
