use tempfile::TempDir;

use crate::anchor::{
    AnchorObservation, AnchorProbe, AnchorSet, ContentDigest, FileAnchor, RepoPath,
};
use crate::embedder::HashEmbedder;
use crate::service::MemoryService;
use crate::DEFAULT_DIMENSION;

fn service() -> (TempDir, MemoryService<HashEmbedder>) {
    let dir = TempDir::new().expect("tempdir");
    let svc =
        MemoryService::open(dir.path(), HashEmbedder::new(DEFAULT_DIMENSION)).expect("open store");
    (dir, svc)
}

/// A probe that records whether the generation gate was free while it
/// looked: file I/O under the gate would hold up every generation writer —
/// and, behind a queued writer, every reader of the daemon.
struct GateWitness<'a> {
    svc: &'a MemoryService<HashEmbedder>,
    looked_with_gate_held: bool,
    looks: usize,
}

/// The checkout root the gate test's anchors are recorded under.
const ROOT: &str = "/work/repo";

impl AnchorProbe for GateWitness<'_> {
    fn root(&self) -> &str {
        ROOT
    }

    fn observe(&mut self, _anchor: &FileAnchor) -> AnchorObservation {
        self.looks += 1;
        if !self.svc.generation_gate.is_free() {
            self.looked_with_gate_held = true;
        }
        AnchorObservation::Unchanged
    }
}

/// Given an anchored lesson, when it is recalled with its anchors judged,
/// then the probe looks at the files after the generation gate is released.
#[test]
fn anchored_recall_judges_freshness_outside_the_generation_gate() {
    let (_dir, svc) = service();
    let anchors = AnchorSet {
        root: ROOT.to_owned(),
        files: vec![FileAnchor {
            path: RepoPath::parse("src/lib.rs").unwrap(),
            digest: ContentDigest::of(b"fn main() {}\n"),
        }],
    };
    svc.remember_anchored("anchored lesson", &[], None, None, Some(&anchors))
        .expect("anchored remember");
    let mut probe = GateWitness {
        svc: &svc,
        looked_with_gate_held: false,
        looks: 0,
    };
    let recalled = svc
        .recall_anchored("anchored lesson", 1, None, &mut probe)
        .expect("anchored recall");
    assert_eq!(recalled.len(), 1);
    assert_eq!(probe.looks, 1, "the anchored file was looked at");
    assert!(
        !probe.looked_with_gate_held,
        "freshness was judged while the generation gate was held"
    );
}
