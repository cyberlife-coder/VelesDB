//! `velesdb db snapshot|restore` — offline directory copies.
//!
//! A snapshot is a plain copy of the database directory plus a
//! `snapshot.manifest.json`: `snapshot` opens the database (taking its
//! exclusive `velesdb.lock`, so a running server makes it fail rather than
//! copy a live tree), flushes every collection, copies the tree, and writes
//! the manifest LAST — its presence is the completeness marker `restore`
//! checks. Both sides are copy-only: no in-place overwrite, ever.

use std::io;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::maintenance::dir_size;

/// Written as the last step of a snapshot; its presence = complete copy.
pub(crate) const MANIFEST: &str = "snapshot.manifest.json";
const MANIFEST_FORMAT: u64 = 1;
/// Live-lock file, recreated by the next `Database::open`; never copied.
const LOCK_FILE: &str = "velesdb.lock";

/// Handles `db snapshot`: flush + copy `src` into a fresh `dst` directory.
pub fn handle_db_snapshot(src: &Path, dst: &Path) -> Result<()> {
    check_destination(dst)?;
    let dst_canonical = canonical_parent(dst)?;
    let src_canonical = src
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", src.display()))?;
    if dst_canonical.starts_with(&src_canonical) {
        bail!("refusing to snapshot a database into its own directory");
    }
    let db = crate::helpers::open_database(src)?;
    let failures = db.flush_all();
    if failures > 0 {
        bail!("snapshot aborted: {failures} collection(s) failed to flush");
    }
    let collections = db.list_collections();
    let files = copy_tree(src, dst)?;
    let bytes = dir_size(dst)?;
    let manifest = serde_json::json!({
        "format": MANIFEST_FORMAT,
        "created_unix_ms": chrono::Utc::now().timestamp_millis(),
        "source": src_canonical.display().to_string(),
        "collections": collections,
        "files": files,
        "bytes": bytes,
        "note": "written last; a snapshot without this manifest is incomplete",
    });
    std::fs::write(dst.join(MANIFEST), serde_json::to_vec_pretty(&manifest)?)?;
    println!(
        "Snapshot: {} → {} ({} file(s), {} bytes, {} collection(s))",
        src.display(),
        dst.display(),
        files,
        bytes,
        collections.len()
    );
    Ok(())
}

/// A destination that exists must be an empty directory.
fn check_destination(dst: &Path) -> Result<()> {
    match dst.try_exists() {
        Ok(false) => {}
        Ok(true) if dst.is_dir() => {
            if std::fs::read_dir(dst)?.next().is_some() {
                bail!("destination is not empty: {}", dst.display());
            }
        }
        Ok(true) => bail!("destination is not a directory: {}", dst.display()),
        Err(e) => return Err(e).with_context(|| format!("cannot inspect {}", dst.display())),
    }
    Ok(())
}

/// The destination with its parent resolved, so a not-yet-existing directory
/// can still be compared against the source tree.
fn canonical_parent(dst: &Path) -> Result<PathBuf> {
    // `Path::parent` of a bare file name is `""`; treat it as cwd.
    let parent = dst.parent().map_or_else(
        || Path::new(".").canonicalize(),
        |p| {
            if p.as_os_str().is_empty() {
                Path::new(".").canonicalize()
            } else {
                p.canonicalize()
            }
        },
    );
    let parent =
        parent.with_context(|| format!("destination parent does not exist: {}", dst.display()))?;
    Ok(parent.join(dst.file_name().unwrap_or_default()))
}

/// Copies `src` into `dst` recursively, regular files only, without following
/// symlinks (a link could point outside the tree). `velesdb.lock` and the
/// manifest are skipped. Returns the file count.
fn copy_tree(src: &Path, dst: &Path) -> Result<u64> {
    std::fs::create_dir(dst)?;
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(src)? {
        entries.push(entry?.file_name());
    }
    entries.sort();
    let mut count = 0;
    for name in entries {
        count += copy_entry(&src.join(&name), &dst.join(&name))?;
    }
    Ok(count)
}

fn copy_entry(src: &Path, dst: &Path) -> Result<u64> {
    let kind = std::fs::symlink_metadata(src)?.file_type();
    if kind.is_symlink() {
        bail!(
            "refusing to copy symlink inside a database: {}",
            src.display()
        );
    }
    if kind.is_dir() {
        return copy_tree(src, dst);
    }
    if !kind.is_file()
        || src
            .file_name()
            .is_some_and(|n| n == LOCK_FILE || n == MANIFEST)
    {
        return Ok(0);
    }
    std::fs::copy(src, dst)?;
    Ok(1)
}

/// Handles `db restore`: verifies the manifest, copies the snapshot into a
/// fresh directory, then opens it to prove it loads.
pub fn handle_db_restore(src: &Path, dst: &Path) -> Result<()> {
    let manifest = read_manifest(src)?;
    check_destination(dst)?;
    copy_tree(src, dst)?;
    println!(
        "Restored {} file(s) from {} → {}",
        manifest["files"].as_u64().unwrap_or_default(),
        src.display(),
        dst.display()
    );
    let db = crate::helpers::open_database(dst)
        .context("copied, but the restored directory does not open as a database")?;
    println!(
        "Verified: {} collection(s) load",
        db.list_collections().len()
    );
    Ok(())
}

fn read_manifest(src: &Path) -> Result<serde_json::Value> {
    let bytes = std::fs::read(src.join(MANIFEST)).map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            anyhow::anyhow!(
                "{} is not a velesdb snapshot ({MANIFEST} missing — snapshots are \
                 written last, so this copy is incomplete or was never made)",
                src.display()
            )
        } else {
            anyhow::anyhow!(e)
        }
    })?;
    let manifest: serde_json::Value = serde_json::from_slice(&bytes)?;
    if manifest["format"].as_u64() != Some(MANIFEST_FORMAT) {
        bail!("unsupported snapshot format in {}", src.display());
    }
    Ok(manifest)
}
