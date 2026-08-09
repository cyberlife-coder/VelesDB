//! The export contract: complete, policy-consistent with `list_memories` —
//! and above all EMBEDDER-FREE, so a store the daemon refuses to serve is
//! still the user's to read.

use super::export_jsonl;
use crate::embedding_provenance::{self, EmbeddingProvenance};
use crate::{HashEmbedder, MemoryService, DEFAULT_DIMENSION};

fn seeded_store() -> (tempfile::TempDir, u64) {
    let dir = tempfile::tempdir().expect("tempdir");
    let service =
        MemoryService::open(dir.path(), HashEmbedder::new(DEFAULT_DIMENSION)).expect("open store");
    let id = service
        .remember(
            "le timeout est de 8 secondes",
            &[],
            Some(&serde_json::Map::from_iter([(
                "project".to_owned(),
                serde_json::json!("acme"),
            )])),
        )
        .expect("remember");
    service
        .remember("le port est 6333", &[], None)
        .expect("remember");
    (dir, id)
}

#[test]
fn every_fact_comes_out_with_content_and_metadata() {
    let (dir, id) = seeded_store();
    let mut buffer = Vec::new();
    let written = export_jsonl(dir.path(), &mut buffer, false).expect("export");
    assert_eq!(written, 2, "both facts are exported");

    let lines: Vec<serde_json::Value> = String::from_utf8(buffer)
        .expect("utf8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is one JSON object"))
        .collect();
    assert_eq!(lines.len(), 2);
    let exported = lines
        .iter()
        .find(|line| line["id"] == id)
        .expect("the metadata-carrying fact is present");
    assert_eq!(exported["content"], "le timeout est de 8 secondes");
    assert_eq!(exported["metadata"]["project"], "acme");
    assert_eq!(
        exported["id_str"],
        id.to_string(),
        "the decimal-string twin rides every line (issue #1468)"
    );
}

#[test]
fn a_store_the_daemon_refuses_still_exports() {
    // The audit scenario's dead end, removed: record a DIFFERENT model as
    // the store's provenance, so the daemon's pre-open check would refuse —
    // and assert the export neither checks nor cares. Your data outlives
    // your configuration.
    let (dir, _) = seeded_store();
    embedding_provenance::write(
        dir.path(),
        &EmbeddingProvenance::new("some-other-model", 4096),
    )
    .expect("plant a provenance the daemon would refuse");

    let mut buffer = Vec::new();
    let written = export_jsonl(dir.path(), &mut buffer, false)
        .expect("the export must not consult the embedder or its provenance");
    assert_eq!(written, 2, "the refused store still yields all its facts");
}
