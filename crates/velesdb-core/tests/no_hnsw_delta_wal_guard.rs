//! Guard test against reintroduction of the dead `hnsw_delta_wal` module.
//!
//! Task 14.1 removed `crates/velesdb-core/src/storage/hnsw_delta_wal.rs` and its
//! test module because the delta-WAL was dead-in-core (never wired into the
//! recovery/open/flush path). The disposition is recorded in
//! `docs/CONCURRENCY_MODEL.md` ("HNSW Delta WAL"): O(delta) fast recovery is
//! relocated to premium via the `WalCursor` seam (Requirement 6 / Task 9).
//!
//! This test fails if the module silently reappears anywhere in the core `src`
//! tree, forcing any reintroduction to be an explicit, reviewed decision rather
//! than an accidental resurrection.
//!
//! _Requirements: 11.3_

use std::fs;
use std::path::{Path, PathBuf};

/// Marker directing a failing reader to the recorded disposition.
const DISPOSITION_NOTE: &str = "The `hnsw_delta_wal` module was intentionally removed from \
velesdb-core (Task 14, Requirement 11). O(delta) fast recovery belongs in premium via the \
WalCursor seam. See the \"HNSW Delta WAL\" section of docs/CONCURRENCY_MODEL.md before \
reintroducing it. If you truly need it back in core, record an explicit decision there and \
update this guard.";

fn core_src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Recursively collect every `.rs` file under `dir`.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => panic!("failed to read directory {}: {e}", dir.display()),
    };
    for entry in entries {
        let path = entry.expect("read dir entry").path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
    out
}

#[test]
fn no_hnsw_delta_wal_source_file_exists() {
    let src = core_src_dir();
    let offenders: Vec<PathBuf> = rust_files(&src)
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| name.starts_with("hnsw_delta_wal"))
        })
        .collect();

    assert!(
        offenders.is_empty(),
        "Found resurrected `hnsw_delta_wal` source file(s): {offenders:?}\n{DISPOSITION_NOTE}"
    );
}

#[test]
fn no_module_declares_hnsw_delta_wal() {
    let src = core_src_dir();
    let mut offenders: Vec<String> = Vec::new();

    for path in rust_files(&src) {
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(e) => panic!("failed to read {}: {e}", path.display()),
        };
        for (idx, line) in contents.lines().enumerate() {
            if line.contains("mod hnsw_delta_wal") {
                offenders.push(format!("{}:{}: {}", path.display(), idx + 1, line.trim()));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "Found module declaration(s) for `hnsw_delta_wal`:\n{}\n{DISPOSITION_NOTE}",
        offenders.join("\n")
    );
}
