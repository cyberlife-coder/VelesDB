//! Feature: a lesson anchored to files goes stale when those files change.
//!
//! An agent remembers "how this module handles errors" against the bytes of
//! the module it read. Months later the module is rewritten. Recall must say
//! so — mark the lesson stale, name the file that moved, and rank it below a
//! fresh lesson of comparable relevance — instead of serving the old advice
//! with full confidence.

#![cfg(feature = "persistence")]

mod common;

use std::fs;
use std::path::PathBuf;

use common::service;
use tempfile::TempDir;
use velesdb_memory::anchor::{AnchorRoot, FileAnchor, Freshness, RepoPath, MAX_ANCHORS_PER_FACT};
use velesdb_memory::{HashEmbedder, MemoryError, MemoryService};

const LESSON: &str = "the parser module reports errors through ParseError, never by panicking";
const RIVAL: &str = "the parser module reports errors through a typed error and never panics";

/// A checkout under an allowlisted directory, with one source file.
struct Checkout {
    _dir: TempDir,
    allowlist: Vec<PathBuf>,
    repo: PathBuf,
}

impl Checkout {
    fn new() -> Self {
        let dir = TempDir::new().expect("tempdir");
        let allowed = fs::canonicalize(dir.path()).expect("canonical tempdir");
        let repo = allowed.join("repo");
        fs::create_dir_all(repo.join("src")).expect("checkout");
        fs::write(
            repo.join("src/parser.rs"),
            "fn parse() -> Result<(), ParseError>",
        )
        .expect("file");
        Self {
            _dir: dir,
            allowlist: vec![allowed],
            repo,
        }
    }

    fn root(&self) -> AnchorRoot {
        AnchorRoot::open(self.repo.to_str().expect("utf-8 path"), &self.allowlist)
            .expect("root inside the allowlist")
    }

    fn anchor(&self, path: &str) -> FileAnchor {
        self.root().anchor(path).expect("anchorable file")
    }

    fn edit(&self, content: &str) {
        fs::write(self.repo.join("src/parser.rs"), content).expect("edit");
    }
}

fn remember_both(svc: &MemoryService<HashEmbedder>, anchors: &[FileAnchor]) -> (u64, u64) {
    let lesson = svc
        .remember_anchored(LESSON, &[], None, None, anchors)
        .expect("anchored remember");
    let rival = svc.remember(RIVAL, &[], None).expect("rival");
    (lesson, rival)
}

fn recall_ids(svc: &MemoryService<HashEmbedder>, checkout: &Checkout) -> Vec<(u64, Freshness)> {
    svc.recall_anchored(LESSON, 2, None, &mut checkout.root())
        .expect("anchored recall")
        .into_iter()
        .map(|hit| (hit.recollection.id, hit.freshness))
        .collect()
}

/// Given a lesson anchored to `src/parser.rs`,
/// when the file is unchanged, then the lesson is fresh and keeps its rank;
/// when the file is rewritten, then the lesson is stale, names the file, and
/// drops below the fresh rival;
/// when the file is restored, then the lesson is fresh again.
#[test]
fn an_edited_anchor_marks_the_lesson_stale_and_down_weights_it() {
    let (_store, svc) = service();
    let checkout = Checkout::new();
    let (lesson, rival) = remember_both(&svc, &[checkout.anchor("src/parser.rs")]);

    let fresh = recall_ids(&svc, &checkout);
    assert_eq!(
        fresh[0],
        (lesson, Freshness::Fresh),
        "exact match leads while fresh"
    );
    assert_eq!(fresh[1], (rival, Freshness::Unanchored));

    checkout.edit("fn parse() { panic!(\"rewritten\") }");
    let stale = recall_ids(&svc, &checkout);
    assert_eq!(stale[0].0, rival, "a stale lesson yields to a fresh one");
    assert_eq!(
        stale[1],
        (
            lesson,
            Freshness::Stale {
                changed: vec![RepoPath::parse("src/parser.rs").unwrap()]
            }
        )
    );

    checkout.edit("fn parse() -> Result<(), ParseError>");
    assert_eq!(recall_ids(&svc, &checkout)[0], (lesson, Freshness::Fresh));
}

/// Given an anchored lesson, when its file is deleted, then it is stale.
#[test]
fn a_deleted_anchor_marks_the_lesson_stale() {
    let (_store, svc) = service();
    let checkout = Checkout::new();
    let (lesson, _) = remember_both(&svc, &[checkout.anchor("src/parser.rs")]);
    fs::remove_file(checkout.repo.join("src/parser.rs")).expect("delete");
    let hits = recall_ids(&svc, &checkout);
    let verdict = hits
        .iter()
        .find(|(id, _)| *id == lesson)
        .map(|(_, f)| f.clone());
    assert!(
        matches!(verdict, Some(Freshness::Stale { .. })),
        "{verdict:?}"
    );
}

/// Anchors are system state: they never leak into caller metadata, and plain
/// `recall` neither reads files nor changes its order because of them.
#[test]
fn anchors_stay_out_of_metadata_and_out_of_plain_recall() {
    let (_store, svc) = service();
    let checkout = Checkout::new();
    let (lesson, _) = remember_both(&svc, &[checkout.anchor("src/parser.rs")]);
    checkout.edit("rewritten");
    let plain = svc.recall(LESSON, 2, None).expect("plain recall");
    assert_eq!(plain[0].id, lesson, "plain recall does not probe anchors");
    let metadata = plain[0].metadata.clone().unwrap_or_default();
    assert!(
        metadata.keys().all(|key| !key.contains("anchor")),
        "anchors are reserved: {metadata:?}"
    );
}

/// Given a stale lesson that was re-verified, when it is remembered again
/// with its anchors, then it is fresh against the new bytes.
#[test]
fn remembering_again_with_anchors_re_anchors_the_lesson() {
    let (_store, svc) = service();
    let checkout = Checkout::new();
    let (lesson, _) = remember_both(&svc, &[checkout.anchor("src/parser.rs")]);
    checkout.edit("fn parse() -> Result<Ast, ParseError>");
    svc.remember_anchored(LESSON, &[], None, None, &[checkout.anchor("src/parser.rs")])
        .expect("re-anchor");
    assert_eq!(recall_ids(&svc, &checkout)[0], (lesson, Freshness::Fresh));
}

/// Remembering the same fact again WITHOUT anchors keeps the anchors it had:
/// anchoring is learned state, carried forward like confidence.
#[test]
fn remembering_again_without_anchors_keeps_them() {
    let (_store, svc) = service();
    let checkout = Checkout::new();
    let (lesson, _) = remember_both(&svc, &[checkout.anchor("src/parser.rs")]);
    svc.remember(LESSON, &[], None).expect("re-remember");
    checkout.edit("rewritten");
    let verdict = recall_ids(&svc, &checkout)
        .into_iter()
        .find(|(id, _)| *id == lesson)
        .map(|(_, freshness)| freshness);
    assert!(
        matches!(verdict, Some(Freshness::Stale { .. })),
        "{verdict:?}"
    );
}

/// Too many anchors are refused before anything is stored.
#[test]
fn too_many_anchors_are_refused_and_nothing_is_stored() {
    let (_store, svc) = service();
    let checkout = Checkout::new();
    let anchors = vec![checkout.anchor("src/parser.rs"); MAX_ANCHORS_PER_FACT + 1];
    let refused = svc.remember_anchored(LESSON, &[], None, None, &anchors);
    assert!(
        matches!(refused, Err(MemoryError::Anchor(_))),
        "{refused:?}"
    );
    assert!(svc.recall(LESSON, 5, None).expect("recall").is_empty());
}
