//! Keeps the embedded SPA in step with `apps/web/dist`.
//!
//! `rust-embed` bakes the frontend into the binary at compile time and emits no
//! change tracking of its own, so a `cargo build` that runs after `npm run
//! build` can reuse a cached crate and ship the previous UI. Listing the emitted
//! assets here makes Cargo rerun this script, and rewriting a stamp file makes
//! the crate itself rebuild — without that second step the embed stays cached.

use std::path::Path;
use std::time::SystemTime;

const DIST_ASSETS: &str = "../../apps/web/dist/assets";
const DIST_INDEX: &str = "../../apps/web/dist/index.html";
const SPA_SOURCE: &str = "../../apps/web/src";
/// Rewritten whenever the embedded assets must be re-read.
const STAMP: &str = "src/assets.stamp";

fn newest_mtime(dir: &Path) -> Option<SystemTime> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut newest: Option<SystemTime> = None;

    for entry in entries.flatten() {
        let path = entry.path();
        let modified = if path.is_dir() {
            newest_mtime(&path)
        } else {
            entry.metadata().ok().and_then(|meta| meta.modified().ok())
        };

        if let Some(modified) = modified {
            newest = Some(match newest {
                Some(current) if current >= modified => current,
                _ => modified,
            });
        }
    }

    newest
}

/// Detects a rebuilt SPA: the emitted asset filenames are content-hashed.
fn asset_listing() -> String {
    let mut names: Vec<String> = std::fs::read_dir(DIST_ASSETS)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    names.sort();
    names.join("\n")
}

fn main() {
    println!("cargo:rerun-if-changed={DIST_INDEX}");
    println!("cargo:rerun-if-changed={DIST_ASSETS}");
    println!("cargo:rerun-if-changed={SPA_SOURCE}");
    println!("cargo:rerun-if-changed={STAMP}");

    // A workspace checked out before the SPA is built still has to compile;
    // `assets.rs` tolerates a missing dist directory.
    let Ok(dist_modified) = Path::new(DIST_INDEX)
        .metadata()
        .and_then(|meta| meta.modified())
    else {
        return;
    };

    if let Some(newest_source) = newest_mtime(Path::new(SPA_SOURCE)) {
        if newest_source > dist_modified {
            println!(
                "cargo:warning=apps/web/dist is older than apps/web/src; the binary will embed a stale SPA. Run `npm run build` (or `make build`) first."
            );
        }
    }

    let listing = asset_listing();
    let stamp = Path::new(STAMP);
    let previous = std::fs::read_to_string(stamp).unwrap_or_default();

    if previous != listing {
        let _ = std::fs::write(stamp, listing);
    }
}
