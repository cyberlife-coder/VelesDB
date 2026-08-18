use super::query_error;
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Component, Path, PathBuf};

const FINGERPRINT_DOMAIN: &[u8] = b"velesdb-migration-source-tree-v2\0";

#[derive(Debug, Clone, Copy)]
enum EntryKind {
    Directory,
    File { len: u64 },
}

#[derive(Debug)]
struct TreeEntry {
    relative_path: PathBuf,
    kind: EntryKind,
}

/// A versioned SHA-256 digest of every directory, regular file and file byte.
///
/// Paths, entry kinds and lengths are length-delimited before they are hashed,
/// so two different trees cannot become ambiguous through path concatenation.
/// Symlinks and special files are refused: following one would let a migration
/// fingerprint or copy data outside the source tree.
///
/// # Errors
/// Returns [`crate::MemoryError`] if the tree cannot be walked or read, or if
/// it contains anything other than directories and regular files.
pub fn fingerprint(root: &Path) -> Result<String, crate::MemoryError> {
    let entries = tree_entries(root)?;
    let mut hash = Sha256::new();
    hash.update(FINGERPRINT_DOMAIN);
    hash.update(
        u64::try_from(entries.len())
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );

    for entry in entries {
        match entry.kind {
            EntryKind::Directory => hash.update([b'd']),
            EntryKind::File { len } => {
                hash.update([b'f']);
                hash.update(len.to_le_bytes());
            }
        }
        hash_relative_path(&mut hash, &entry.relative_path);
        if let EntryKind::File { len } = entry.kind {
            hash_file(root, &entry.relative_path, len, &mut hash)?;
        }
    }

    Ok(format!("sha256-tree-v2:{}", encode_hex(&hash.finalize())))
}

/// Sum of every regular file's length under `root`.
///
/// # Errors
/// Returns [`crate::MemoryError`] under the same conditions as [`fingerprint`].
pub fn bytes_on_disk(root: &Path) -> Result<u64, crate::MemoryError> {
    tree_entries(root)?
        .into_iter()
        .try_fold(0u64, |total, entry| {
            let len = match entry.kind {
                EntryKind::Directory => 0,
                EntryKind::File { len } => len,
            };
            total.checked_add(len).ok_or_else(|| {
                query_error(format!(
                    "the byte count under {} exceeds u64",
                    root.display()
                ))
            })
        })
}

fn tree_entries(root: &Path) -> Result<Vec<TreeEntry>, crate::MemoryError> {
    let metadata = std::fs::symlink_metadata(root)
        .map_err(|err| query_error(format!("cannot inspect source {}: {err}", root.display())))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(query_error(format!(
            "migration source {} must be a real directory, not a symlink or special file",
            root.display()
        )));
    }

    let mut entries = Vec::new();
    collect_entries(root, root, &mut entries)?;
    entries.sort_unstable_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(entries)
}

fn collect_entries(
    root: &Path,
    directory: &Path,
    entries: &mut Vec<TreeEntry>,
) -> Result<(), crate::MemoryError> {
    let read = std::fs::read_dir(directory)
        .map_err(|err| query_error(format!("cannot read {}: {err}", directory.display())))?;
    for entry in read {
        let entry = entry.map_err(|err| query_error(format!("cannot read an entry: {err}")))?;
        collect_entry(root, &entry.path(), entries)?;
    }
    Ok(())
}

fn collect_entry(
    root: &Path,
    path: &Path,
    entries: &mut Vec<TreeEntry>,
) -> Result<(), crate::MemoryError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|err| query_error(format!("cannot inspect {}: {err}", path.display())))?;
    let relative_path = path
        .strip_prefix(root)
        .map_err(|err| query_error(format!("cannot relativize {}: {err}", path.display())))?
        .to_path_buf();
    if metadata.file_type().is_symlink() {
        return Err(query_error(format!(
            "migration source contains symlink {}; refusing to follow data outside the tree",
            path.display()
        )));
    }
    if metadata.is_dir() {
        entries.push(TreeEntry {
            relative_path,
            kind: EntryKind::Directory,
        });
        return collect_entries(root, path, entries);
    }
    if metadata.is_file() {
        entries.push(TreeEntry {
            relative_path,
            kind: EntryKind::File {
                len: metadata.len(),
            },
        });
        return Ok(());
    }
    Err(query_error(format!(
        "migration source contains special file {}; only directories and regular files are supported",
        path.display()
    )))
}

fn hash_relative_path(hash: &mut Sha256, path: &Path) {
    let components: Vec<_> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part),
            _ => None,
        })
        .collect();
    hash.update(
        u64::try_from(components.len())
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    for component in components {
        update_os_str(hash, component);
    }
}

fn hash_file(
    root: &Path,
    relative_path: &Path,
    expected_len: u64,
    hash: &mut Sha256,
) -> Result<(), crate::MemoryError> {
    let path = root.join(relative_path);
    let file = open_expected_file(&path, expected_len)?;
    let bytes_read = hash_reader(&path, file, hash)?;
    if bytes_read == expected_len {
        return Ok(());
    }
    Err(query_error(format!(
        "source changed while fingerprinting {}: expected {expected_len} bytes, read {bytes_read}",
        path.display()
    )))
}

fn open_expected_file(path: &Path, expected_len: u64) -> Result<File, crate::MemoryError> {
    let file = File::open(path)
        .map_err(|err| query_error(format!("cannot open {}: {err}", path.display())))?;
    let before = file
        .metadata()
        .map_err(|err| query_error(format!("cannot inspect {}: {err}", path.display())))?;
    if before.is_file() && before.len() == expected_len {
        return Ok(file);
    }
    Err(query_error(format!(
        "source changed while fingerprinting {}",
        path.display()
    )))
}

fn hash_reader(path: &Path, file: File, hash: &mut Sha256) -> Result<u64, crate::MemoryError> {
    let mut reader = BufReader::new(file);
    let mut buffer = vec![0u8; 64 * 1024].into_boxed_slice();
    let mut bytes_read = 0u64;
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|err| query_error(format!("cannot read {}: {err}", path.display())))?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
        bytes_read = bytes_read
            .checked_add(u64::try_from(read).unwrap_or(u64::MAX))
            .ok_or_else(|| query_error(format!("file {} exceeds u64", path.display())))?;
    }
    Ok(bytes_read)
}

#[cfg(unix)]
fn update_os_str(hash: &mut Sha256, value: &OsStr) {
    use std::os::unix::ffi::OsStrExt;
    let bytes = value.as_bytes();
    hash.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_le_bytes());
    hash.update(bytes);
}

#[cfg(windows)]
fn update_os_str(hash: &mut Sha256, value: &OsStr) {
    use std::os::windows::ffi::OsStrExt;

    hash.update(
        u64::try_from(value.encode_wide().count())
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    for unit in value.encode_wide() {
        hash.update(unit.to_le_bytes());
    }
}

#[cfg(not(any(unix, windows)))]
fn update_os_str(hash: &mut Sha256, value: &OsStr) {
    let bytes = value.to_string_lossy();
    hash.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_le_bytes());
    hash.update(bytes.as_bytes());
}

pub(super) fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(all(test, windows))]
#[path = "filesystem_tests.rs"]
mod windows_tests;
