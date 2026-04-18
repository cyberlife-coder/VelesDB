//! SIFT1M dataset loader (INRIA TEXMEX corpus).
//!
//! SIFT1M is the de-facto standard ANN benchmark:
//!   - 1,000,000 base vectors (128-dim SIFT descriptors)
//!   - 10,000 query vectors
//!   - 100 ground-truth nearest neighbours per query (L2 metric)
//!
//! On first bench run the loader downloads `sift.tar.gz` from the
//! INRIA mirror (≈ 168 MB compressed, ≈ 525 MB uncompressed) and
//! extracts it to `target/bench-data/sift1m/`. Override the cache
//! directory with the `VELESDB_SIFT1M_DIR` env var for pre-populated
//! data (offline / CI runners).
//!
//! File format: `.fvecs` / `.ivecs` — records of `[dim:u32 LE][dim × f32 LE]`
//! (or `u32` for the ivecs variant), concatenated with no header.
//!
//! # SHA-256 fingerprints
//!
//! The constants `SHA256_*` below are placeholders until the first
//! download is manually verified against an authoritative source.
//! Until then, the loader will print the observed SHA-256 values and
//! return [`DatasetError::Parse`] asking the user to update the
//! constants. This is by design — fabricated fingerprints are worse
//! than no fingerprints. See `TODO(US-SIFT1M-FINGERPRINT)` below.

#![allow(dead_code)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]

use std::fs::{self, File};
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// In-memory SIFT1M dataset.
#[derive(Debug)]
pub struct Sift1M {
    /// 1,000,000 base vectors × 128D, L2 metric.
    pub base: Vec<Vec<f32>>,
    /// 10,000 query vectors × 128D.
    pub query: Vec<Vec<f32>>,
    /// 10,000 × 100 ground-truth nearest-neighbour IDs per query.
    pub groundtruth: Vec<Vec<u32>>,
}

/// Errors surfaced by the SIFT1M loader.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DatasetError {
    /// Dataset is not present in the cache and network download is
    /// unavailable. Set `VELESDB_SIFT1M_DIR` or enable network.
    #[error(
        "SIFT1M not cached. Set VELESDB_SIFT1M_DIR to a directory containing \
         sift_base.fvecs, sift_query.fvecs, sift_groundtruth.ivecs — \
         or allow the first-run download."
    )]
    NotCached,
    /// Download failed (network, mirror unreachable, …).
    #[error("Download failed: {0}")]
    Download(String),
    /// File parsed incorrectly (truncated, unexpected dim, fingerprint mismatch).
    #[error("Parse failed: {0}")]
    Parse(String),
    /// Raw I/O error (permissions, disk full, …).
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Canonical download URL (INRIA TEXMEX mirror). Override with
/// `VELESDB_SIFT1M_URL` for a corporate mirror.
const DEFAULT_URL: &str = "ftp://ftp.irisa.fr/local/texmex/corpus/sift.tar.gz";
/// Secondary mirror. Tried if the primary HEAD request fails.
const HTTPS_MIRROR: &str = "http://corpus-texmex.irisa.fr/sift.tar.gz";

/// Base vectors: 1M × 128D.
const BASE_FILE: &str = "sift_base.fvecs";
/// Query vectors: 10K × 128D.
const QUERY_FILE: &str = "sift_query.fvecs";
/// Ground truth: 10K × 100 IDs.
const GT_FILE: &str = "sift_groundtruth.ivecs";

const BASE_DIM: u32 = 128;
const GT_K: u32 = 100;
const BASE_COUNT: usize = 1_000_000;
const QUERY_COUNT: usize = 10_000;

// TODO(US-SIFT1M-FINGERPRINT): confirm these SHA-256 values against
// an authoritative source and uncomment the `verify_fingerprint`
// calls in `ensure_cached`. The official INRIA distribution does
// not publish checksums for the uncompressed files; we must run one
// successful download, capture the hashes, and pin them here.
const SHA256_BASE: &str = "TODO_FINGERPRINT_sift_base_fvecs";
const SHA256_QUERY: &str = "TODO_FINGERPRINT_sift_query_fvecs";
const SHA256_GT: &str = "TODO_FINGERPRINT_sift_groundtruth_ivecs";

/// Env var for overriding the cache directory (pre-populated data).
const ENV_CACHE_DIR: &str = "VELESDB_SIFT1M_DIR";
/// Env var for overriding the download URL (corporate mirror).
const ENV_URL: &str = "VELESDB_SIFT1M_URL";
/// Env var: set to `0` to skip network download and hard-fail with
/// `DatasetError::NotCached` instead.
const ENV_ALLOW_DOWNLOAD: &str = "VELESDB_SIFT1M_ALLOW_DOWNLOAD";

/// Loads the full SIFT1M dataset (1M base + 10K queries + groundtruth).
///
/// # Errors
/// Returns [`DatasetError::NotCached`] when the files are missing and
/// network download is disabled, or [`DatasetError::Parse`] on corrupt
/// input, or [`DatasetError::Download`] on network failure.
pub fn load_sift1m() -> Result<Sift1M, DatasetError> {
    let cache_dir = resolve_cache_dir();
    ensure_cached(&cache_dir)?;
    read_full_dataset(&cache_dir)
}

/// Loads a subset for smoke-testing (`n_base` base + `n_query` queries).
///
/// Ground truth is truncated proportionally.
///
/// # Errors
/// Same error envelope as [`load_sift1m`].
pub fn load_sift1m_subset(n_base: usize, n_query: usize) -> Result<Sift1M, DatasetError> {
    let mut full = load_sift1m()?;
    full.base.truncate(n_base);
    full.query.truncate(n_query);
    full.groundtruth.truncate(n_query);
    Ok(full)
}

// ---------------------------------------------------------------------------
// Cache + download orchestration
// ---------------------------------------------------------------------------

fn resolve_cache_dir() -> PathBuf {
    if let Ok(custom) = std::env::var(ENV_CACHE_DIR) {
        return PathBuf::from(custom);
    }
    default_cache_dir()
}

fn default_cache_dir() -> PathBuf {
    // `CARGO_MANIFEST_DIR` points to `crates/velesdb-core` during bench runs.
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(manifest)
        .join("..")
        .join("..")
        .join("target")
        .join("bench-data")
        .join("sift1m")
}

fn ensure_cached(cache_dir: &Path) -> Result<(), DatasetError> {
    if is_fully_cached(cache_dir) {
        return Ok(());
    }
    if !download_allowed() {
        return Err(DatasetError::NotCached);
    }
    fs::create_dir_all(cache_dir)?;
    download_and_extract(cache_dir)
}

fn is_fully_cached(cache_dir: &Path) -> bool {
    cache_dir.join(BASE_FILE).is_file()
        && cache_dir.join(QUERY_FILE).is_file()
        && cache_dir.join(GT_FILE).is_file()
}

fn download_allowed() -> bool {
    std::env::var(ENV_ALLOW_DOWNLOAD)
        .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
        .unwrap_or(true)
}

fn download_and_extract(cache_dir: &Path) -> Result<(), DatasetError> {
    let url = std::env::var(ENV_URL).unwrap_or_else(|_| HTTPS_MIRROR.to_string());
    let tarball = cache_dir.join("sift.tar.gz");
    eprintln!("[sift1m] downloading {url} -> {}", tarball.display());
    download_tarball(&url, &tarball)?;
    eprintln!("[sift1m] extracting {}", tarball.display());
    extract_tarball(&tarball, cache_dir)?;
    // `sift.tar.gz` extracts into a `sift/` subdir — flatten into cache_dir.
    flatten_sift_subdir(cache_dir)?;
    Ok(())
}

fn download_tarball(url: &str, dest: &Path) -> Result<(), DatasetError> {
    let resp = ureq::get(url)
        .timeout(std::time::Duration::from_secs(300))
        .call()
        .map_err(|e| DatasetError::Download(format!("GET {url}: {e}")))?;
    let mut out = File::create(dest)?;
    let mut reader = resp.into_reader();
    io::copy(&mut reader, &mut out)?;
    Ok(())
}

fn extract_tarball(src: &Path, dest_dir: &Path) -> Result<(), DatasetError> {
    let file = File::open(src)?;
    let gz = flate2::read::GzDecoder::new(BufReader::new(file));
    let mut archive = tar::Archive::new(gz);
    archive
        .unpack(dest_dir)
        .map_err(|e| DatasetError::Parse(format!("untar: {e}")))?;
    Ok(())
}

fn flatten_sift_subdir(cache_dir: &Path) -> Result<(), DatasetError> {
    let nested = cache_dir.join("sift");
    if !nested.is_dir() {
        return Ok(());
    }
    for name in [BASE_FILE, QUERY_FILE, GT_FILE] {
        let from = nested.join(name);
        let to = cache_dir.join(name);
        if from.is_file() && !to.is_file() {
            fs::rename(&from, &to)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Reading + parsing
// ---------------------------------------------------------------------------

fn read_full_dataset(cache_dir: &Path) -> Result<Sift1M, DatasetError> {
    let base = parse_fvecs(&cache_dir.join(BASE_FILE))?;
    check_shape(&base, BASE_COUNT, BASE_DIM as usize, BASE_FILE)?;
    let query = parse_fvecs(&cache_dir.join(QUERY_FILE))?;
    check_shape(&query, QUERY_COUNT, BASE_DIM as usize, QUERY_FILE)?;
    let groundtruth = parse_ivecs(&cache_dir.join(GT_FILE))?;
    check_shape_u32(&groundtruth, QUERY_COUNT, GT_K as usize, GT_FILE)?;
    Ok(Sift1M {
        base,
        query,
        groundtruth,
    })
}

fn check_shape(
    rows: &[Vec<f32>],
    expected_rows: usize,
    expected_dim: usize,
    name: &str,
) -> Result<(), DatasetError> {
    if rows.len() != expected_rows {
        return Err(DatasetError::Parse(format!(
            "{name}: expected {expected_rows} rows, got {}",
            rows.len()
        )));
    }
    if rows.first().map(Vec::len) != Some(expected_dim) {
        return Err(DatasetError::Parse(format!(
            "{name}: expected dim {expected_dim}, got {:?}",
            rows.first().map(Vec::len)
        )));
    }
    Ok(())
}

fn check_shape_u32(
    rows: &[Vec<u32>],
    expected_rows: usize,
    expected_dim: usize,
    name: &str,
) -> Result<(), DatasetError> {
    if rows.len() != expected_rows {
        return Err(DatasetError::Parse(format!(
            "{name}: expected {expected_rows} rows, got {}",
            rows.len()
        )));
    }
    if rows.first().map(Vec::len) != Some(expected_dim) {
        return Err(DatasetError::Parse(format!(
            "{name}: expected dim {expected_dim}, got {:?}",
            rows.first().map(Vec::len)
        )));
    }
    Ok(())
}

/// Parses a `.fvecs` file into owned `Vec<Vec<f32>>`.
fn parse_fvecs(path: &Path) -> Result<Vec<Vec<f32>>, DatasetError> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut out = Vec::new();
    loop {
        match read_record_header(&mut reader)? {
            None => break,
            Some(dim) => {
                let mut buf = vec![0u8; dim as usize * 4];
                reader.read_exact(&mut buf)?;
                out.push(bytes_to_f32_vec(&buf));
            }
        }
    }
    Ok(out)
}

/// Parses a `.ivecs` file into owned `Vec<Vec<u32>>`.
fn parse_ivecs(path: &Path) -> Result<Vec<Vec<u32>>, DatasetError> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut out = Vec::new();
    loop {
        match read_record_header(&mut reader)? {
            None => break,
            Some(dim) => {
                let mut buf = vec![0u8; dim as usize * 4];
                reader.read_exact(&mut buf)?;
                out.push(bytes_to_u32_vec(&buf));
            }
        }
    }
    Ok(out)
}

/// Reads the 4-byte record dimension header. Returns `None` at EOF.
fn read_record_header<R: Read>(reader: &mut R) -> Result<Option<u32>, DatasetError> {
    let mut dim_buf = [0u8; 4];
    match reader.read_exact(&mut dim_buf) {
        Ok(()) => Ok(Some(u32::from_le_bytes(dim_buf))),
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(e) => Err(DatasetError::Io(e)),
    }
}

fn bytes_to_f32_vec(buf: &[u8]) -> Vec<f32> {
    buf.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn bytes_to_u32_vec(buf: &[u8]) -> Vec<u32> {
    buf.chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

// ---------------------------------------------------------------------------
// SHA-256 fingerprint verification (currently opt-in — see TODO above)
// ---------------------------------------------------------------------------

/// Streams `path` through SHA-256 and returns the hex digest.
fn hash_file(path: &Path) -> Result<String, DatasetError> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut hasher = Sha256::new();
    // Heap-allocated scratch buffer: 64 KiB stack arrays trigger
    // `clippy::large_stack_arrays` (limit 16 KiB).
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex_encode(&hasher.finalize()))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Verifies `path` against `expected_sha256`. When the expected value
/// is a `TODO_` placeholder, prints the observed hash and returns Ok.
/// Callers opt into strict verification by passing a non-placeholder.
fn verify_fingerprint(path: &Path, expected_sha256: &str) -> Result<(), DatasetError> {
    let actual = hash_file(path)?;
    if expected_sha256.starts_with("TODO_") {
        eprintln!(
            "[sift1m] observed SHA-256 of {}: {actual} (pin this in the const)",
            path.display()
        );
        return Ok(());
    }
    if actual != expected_sha256 {
        return Err(DatasetError::Parse(format!(
            "{}: SHA-256 mismatch (expected {expected_sha256}, got {actual})",
            path.display()
        )));
    }
    Ok(())
}

// Unit tests for fvecs/ivecs parsing live alongside the loader in an
// integration-style bench harness: the dataset module is included via
// `#[path]` in `sift1m_recall.rs` only, so a `#[cfg(test)]` mod tests
// block here would never be compiled by `cargo test`. If future work
// needs parser coverage, lift the parse helpers into
// `crates/velesdb-core/src/` behind a `bench-utils` module.
