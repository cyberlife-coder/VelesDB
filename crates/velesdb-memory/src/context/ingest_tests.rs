//! TDD suite for [`super::resolve_fragments`] and [`super::IngestRoots`]
//! (V2b-1): the security pipeline, its short-circuit ordering, and the
//! round trip of a resolved `path` fragment through the compiler and the
//! memory bridge's `ctx://source` handle.

use std::fs;
use std::path::Path;

use super::{resolve_one, IngestRoots};
use crate::context::model::{CompilePolicy, CompileRequest, ContextFragment};
use crate::context::ContextCompiler;
use crate::embedder::HashEmbedder;
use crate::error::MemoryError;
use crate::limits::{MAX_INGEST_FILES, MAX_INGEST_FILE_BYTES, MAX_TOTAL_INGEST_BYTES};
use crate::service::MemoryService;

const DIM: usize = 384;

fn path_fragment(path: &str) -> ContextFragment {
    ContextFragment {
        id: None,
        content: String::new(),
        path: Some(path.to_owned()),
        kind: None,
        priority: None,
        metadata: None,
        media: None,
    }
}

/// An [`IngestRoots`] allowlisting exactly `dir`.
fn roots_for(dir: &Path) -> IngestRoots {
    let value = std::env::join_paths([dir.as_os_str()])
        .expect("a single absolute path always joins")
        .to_string_lossy()
        .into_owned();
    IngestRoots::parse(&value).expect("tempdir is a valid, existing directory")
}

#[test]
fn ingest_disabled_without_roots() {
    let mut fragments = vec![path_fragment("/does/not/matter.txt")];
    let err = super::resolve_fragments(&mut fragments, None).expect_err("no roots configured");
    assert!(matches!(err, MemoryError::IngestDisabled), "{err:?}");
}

#[test]
fn ingest_rejects_relative_path() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let roots = roots_for(dir.path());
    let mut fragments = vec![path_fragment("relative/path.txt")];
    let err = super::resolve_fragments(&mut fragments, Some(&roots)).expect_err("relative path");
    match err {
        MemoryError::IngestPath(msg) => assert!(msg.contains("relative"), "{msg}"),
        other => panic!("expected IngestPath, got {other:?}"),
    }
}

#[test]
fn ingest_rejects_path_outside_roots() {
    let allowed = tempfile::TempDir::new().expect("tempdir");
    let outside = tempfile::TempDir::new().expect("tempdir");
    let target = outside.path().join("secret.txt");
    fs::write(&target, "top secret").expect("write");

    let roots = roots_for(allowed.path());
    let requested = target.to_string_lossy().into_owned();
    let mut fragments = vec![path_fragment(&requested)];
    let err =
        super::resolve_fragments(&mut fragments, Some(&roots)).expect_err("outside every root");
    match err {
        MemoryError::IngestOutsideRoots(path) => assert_eq!(path, requested),
        other => panic!("expected IngestOutsideRoots, got {other:?}"),
    }
}

#[test]
#[cfg(unix)]
fn ingest_follows_symlink_inside_roots() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let real = dir.path().join("real.txt");
    fs::write(&real, "hello from the real file").expect("write");
    let link = dir.path().join("link.txt");
    std::os::unix::fs::symlink(&real, &link).expect("symlink");

    let roots = roots_for(dir.path());
    let requested = link.to_string_lossy().into_owned();
    let mut fragments = vec![path_fragment(&requested)];
    super::resolve_fragments(&mut fragments, Some(&roots)).expect("symlink stays inside roots");
    assert_eq!(fragments[0].content, "hello from the real file");
    assert!(fragments[0].path.is_none());
}

#[test]
#[cfg(unix)]
fn ingest_rejects_symlink_escaping_roots() {
    let allowed = tempfile::TempDir::new().expect("tempdir");
    let outside = tempfile::TempDir::new().expect("tempdir");
    let target = outside.path().join("secret.txt");
    fs::write(&target, "top secret").expect("write");
    let link = allowed.path().join("escape.txt");
    std::os::unix::fs::symlink(&target, &link).expect("symlink");

    let roots = roots_for(allowed.path());
    let requested = link.to_string_lossy().into_owned();
    let mut fragments = vec![path_fragment(&requested)];
    let err = super::resolve_fragments(&mut fragments, Some(&roots))
        .expect_err("symlink escapes the root");
    match err {
        MemoryError::IngestOutsideRoots(path) => assert_eq!(path, requested),
        other => panic!("expected IngestOutsideRoots, got {other:?}"),
    }
}

#[test]
fn ingest_rejects_directory() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let sub = dir.path().join("subdir");
    fs::create_dir(&sub).expect("mkdir");

    let roots = roots_for(dir.path());
    let requested = sub.to_string_lossy().into_owned();
    let mut fragments = vec![path_fragment(&requested)];
    let err = super::resolve_fragments(&mut fragments, Some(&roots)).expect_err("a directory");
    match err {
        MemoryError::IngestPath(msg) => assert!(msg.contains("not a regular file"), "{msg}"),
        other => panic!("expected IngestPath, got {other:?}"),
    }
}

#[test]
fn ingest_rejects_oversized_file() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let file = dir.path().join("big.txt");
    fs::write(&file, vec![b'a'; MAX_INGEST_FILE_BYTES + 1]).expect("write");

    let roots = roots_for(dir.path());
    let requested = file.to_string_lossy().into_owned();
    let mut fragments = vec![path_fragment(&requested)];
    let err = super::resolve_fragments(&mut fragments, Some(&roots)).expect_err("oversized file");
    match err {
        MemoryError::ContextOverLimit(msg) => {
            assert!(msg.contains(&MAX_INGEST_FILE_BYTES.to_string()), "{msg}");
        }
        other => panic!("expected ContextOverLimit, got {other:?}"),
    }
}

#[test]
fn ingest_rejects_non_utf8_with_binary_hint() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let file = dir.path().join("image.png");
    let mut bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend_from_slice(&[0xFF, 0xFE, 0x00, 0x01]);
    fs::write(&file, &bytes).expect("write");

    let roots = roots_for(dir.path());
    let requested = file.to_string_lossy().into_owned();
    let mut fragments = vec![path_fragment(&requested)];
    let err = super::resolve_fragments(&mut fragments, Some(&roots)).expect_err("non-utf8");
    match err {
        MemoryError::IngestPath(msg) => {
            assert!(msg.contains("not valid UTF-8"), "{msg}");
            assert!(msg.contains("media fragment"), "{msg}");
        }
        other => panic!("expected IngestPath, got {other:?}"),
    }
}

#[test]
fn ingest_rejects_path_plus_content() {
    // No roots configured at all — the shape rule fires first, regardless
    // of whether ingestion is even enabled.
    let mut fragment = path_fragment("/tmp/whatever.txt");
    fragment.content = "already have content".to_owned();
    let mut fragments = vec![fragment];
    let err = super::resolve_fragments(&mut fragments, None).expect_err("path + content");
    match err {
        MemoryError::IngestPath(msg) => assert!(msg.contains("never more than one"), "{msg}"),
        other => panic!("expected IngestPath, got {other:?}"),
    }
}

#[test]
fn ingest_caps_file_count_and_total_bytes() {
    // File-count cap: MAX_INGEST_FILES + 1 fragments, none of which need to
    // resolve to a real file — the count check runs before any filesystem
    // access.
    let dir = tempfile::TempDir::new().expect("tempdir");
    let roots = roots_for(dir.path());
    let mut fragments: Vec<ContextFragment> = (0..=MAX_INGEST_FILES)
        .map(|i| path_fragment(&format!("/nonexistent/{i}.txt")))
        .collect();
    let err = super::resolve_fragments(&mut fragments, Some(&roots))
        .expect_err("too many path fragments");
    match err {
        MemoryError::ContextOverLimit(msg) => {
            assert!(msg.contains(&MAX_INGEST_FILES.to_string()), "{msg}");
        }
        other => panic!("expected ContextOverLimit, got {other:?}"),
    }

    // Aggregate byte cap: white-box call into `resolve_one` with a
    // `total_bytes` accumulator already near the ceiling — with
    // MAX_INGEST_FILES * MAX_INGEST_FILE_BYTES == MAX_TOTAL_INGEST_BYTES,
    // the aggregate cap can never be exceeded by real max-sized files
    // without first tripping the count cap above, so this is the only way
    // to exercise it directly without writing tens of megabytes to disk.
    let file = dir.path().join("small.txt");
    fs::write(&file, b"just a little bit of content").expect("write");
    let requested = file.to_string_lossy().into_owned();
    let mut total_bytes = MAX_TOTAL_INGEST_BYTES - 10;
    let err = resolve_one(&requested, &roots, &mut total_bytes, MAX_INGEST_FILE_BYTES)
        .expect_err("pushes the running total over the aggregate cap");
    match err {
        MemoryError::ContextOverLimit(msg) => {
            assert!(msg.contains(&MAX_TOTAL_INGEST_BYTES.to_string()), "{msg}");
        }
        other => panic!("expected ContextOverLimit, got {other:?}"),
    }
}

#[test]
fn path_fragment_compiles_and_round_trips_source_handle() {
    let store_dir = tempfile::TempDir::new().expect("tempdir");
    let service =
        MemoryService::open(store_dir.path(), HashEmbedder::new(DIM)).expect("open memory store");

    let source_dir = tempfile::TempDir::new().expect("tempdir");
    let file = source_dir.path().join("notes.txt");
    fs::write(&file, "the deploy pipeline is green").expect("write");

    let roots = roots_for(source_dir.path());
    let requested = file.to_string_lossy().into_owned();
    let mut fragments = vec![path_fragment(&requested)];
    super::resolve_fragments(&mut fragments, Some(&roots)).expect("resolves cleanly");

    let request = CompileRequest {
        query: "deploy status".to_owned(),
        fragments,
        project: None,
        target_model: None,
        token_budget: 10_000,
        memory_scope: None,
        policy: Some(CompilePolicy::default()),
    };
    let compiled = service
        .compile_context(&ContextCompiler::new(CompilePolicy::default()), &request)
        .expect("compiles like any other fragment");
    assert!(compiled.content.contains("the deploy pipeline is green"));

    let source = compiled
        .sources
        .first()
        .expect("the resolved fragment has exactly one source");
    let resolved = service
        .retrieve_context_source(&source.handle)
        .expect("the source round-trips");
    assert_eq!(resolved.content, "the deploy pipeline is green");
}

#[test]
#[cfg(unix)]
fn outside_roots_error_never_echoes_resolved_target() {
    let allowed = tempfile::TempDir::new().expect("tempdir");
    let outside = tempfile::TempDir::new().expect("tempdir");
    let target = outside.path().join("only-visible-if-leaked.txt");
    fs::write(&target, "top secret").expect("write");
    let link = allowed.path().join("escape.txt");
    std::os::unix::fs::symlink(&target, &link).expect("symlink");

    let roots = roots_for(allowed.path());
    let requested = link.to_string_lossy().into_owned();
    let mut fragments = vec![path_fragment(&requested)];
    let err = super::resolve_fragments(&mut fragments, Some(&roots))
        .expect_err("symlink escapes the root");
    let message = err.to_string();
    assert!(
        message.contains(&requested),
        "error should cite the requested path: {message}"
    );
    assert!(
        !message.contains("only-visible-if-leaked"),
        "error must never echo the resolved target: {message}"
    );
}
