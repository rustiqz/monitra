//! Builds the web dashboard's static assets before `rust-embed` (`src/assets.rs`)
//! reads them (Phase 10). Node/npm is therefore a build-time prerequisite —
//! documented in the project's `CLAUDE.md` toolchain note alongside the
//! musl one, not silently assumed.
//!
//! Only re-runs when the frontend's own sources change (the
//! `rerun-if-changed` list below), not on every unrelated Rust-only build —
//! `npm ci` re-links `web/node_modules` from scratch each time it runs and
//! is not cheap enough to pay on every `cargo build`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let web_dir = web_dir();

    for path in [
        "src",
        "index.html",
        "package.json",
        "package-lock.json",
        "vite.config.ts",
        "tsconfig.json",
    ] {
        println!("cargo:rerun-if-changed={}", web_dir.join(path).display());
    }

    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };

    run(&web_dir, npm, &["ci"]);
    run(&web_dir, npm, &["run", "build"]);
}

fn web_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web")
}

fn run(dir: &Path, program: &str, args: &[&str]) {
    let status = Command::new(program)
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap_or_else(|source| {
            panic!(
                "monitra-backend/build.rs: failed to run `{program} {}` in {}: {source} — \
                 is Node/npm installed? (see CLAUDE.md toolchain note)",
                args.join(" "),
                dir.display()
            )
        });

    if !status.success() {
        panic!(
            "monitra-backend/build.rs: `{program} {}` in {} exited with {status} — \
             see the npm output above for the actual frontend build error",
            args.join(" "),
            dir.display()
        );
    }
}
