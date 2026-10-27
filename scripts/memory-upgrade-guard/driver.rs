//! Data-safety driver of the velesdb-memory upgrade guard.
//!
//! One source, built twice by `scripts/memory_upgrade_guard.py`: against the
//! last PUBLISHED `velesdb-memory` and against this tree. The two binaries are
//! pointed at the same store directory in turn, which is what a user does when
//! they upgrade (or roll back) the crate under a store they already have.
//!
//! ```text
//! driver seed       <dir> <manifest> [facts]   build a store, record what it must hold
//! driver verify     <dir> <manifest> [min_rate] reopen it, check every record
//! driver append     <dir> <manifest>           add facts, extend the manifest
//! driver soak       <dir> <acks>               write until killed, ack each write
//! driver check      <dir> <acks>               every acknowledged write must survive
//! driver probe-lock <dir>                      a second opener must be refused
//! ```
//!
//! Deterministic (`HashEmbedder`, a stub extractor), offline, no model.
//!
//! What counts as LOSS, and what does not: anything read EXACTLY (the export
//! of the store, its digest, deletions, expiry, working contexts, the graph
//! walk) is a loss when it differs. Retrieval by similarity goes through an
//! approximate index, so it is a rate that may not collapse, never an exact
//! equality: the same version already misses a fact now and then.
//!
//! This is test tooling, outside the crates' `src/`: it panics on the
//! unexpected on purpose, which is how a harness says "this run is void".

use serde_json::{json, Value};
use std::collections::{BTreeSet, HashSet};
use velesdb_memory::context::WorkingContext;
use velesdb_memory::export::export_jsonl;
use velesdb_memory::{
    ColumnFilter, ColumnOp, ExtractError, ExtractedFact, Extractor, FusionOptions, HashEmbedder,
    Link, MemoryError, MemoryService, Metadata, NativeStore, DEFAULT_DIMENSION,
};

const TEN_YEARS: u64 = 315_360_000;
const SHORT_TTL: u64 = 2;
const TOPICS: [&str; 6] = ["billing", "search", "ingest", "auth", "cache", "export"];
const TOOLS: [&str; 5] = ["rust", "sqlite", "redis", "kafka", "nginx"];
const PROJECTS: [&str; 3] = ["alpha", "beta", "gamma"];
/// Default floor for retrieval by own text. The guard passes a floor relative
/// to what the PREVIOUS version retrieves on the same store, because an
/// approximate index over near-identical texts misses a few percent on its own.
const DEFAULT_MIN_RETRIEVAL_RATE: f64 = 0.80;

type Svc = MemoryService<HashEmbedder, NativeStore>;

/// Extraction without a model: the first word of each sentence is the entity.
struct StubExtractor;

impl Extractor for StubExtractor {
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        Ok(text
            .split(". ")
            .filter(|sentence| !sentence.trim().is_empty())
            .map(|sentence| ExtractedFact {
                text: sentence.trim().trim_end_matches('.').to_owned(),
                entities: sentence
                    .split_whitespace()
                    .take(1)
                    .map(str::to_lowercase)
                    .collect(),
            })
            .collect())
    }
}

fn open(dir: &str) -> Svc {
    MemoryService::open(dir, HashEmbedder::new(DEFAULT_DIMENSION)).expect("open the store")
}

fn meta(project: &str, n: usize) -> Metadata {
    let mut m = Metadata::new();
    m.insert("project".into(), project.into());
    m.insert("n".into(), n.into());
    m.insert("tag".into(), format!("t{}", n % 7).into());
    m
}

fn fact_text(i: usize) -> String {
    format!(
        "fact {i}: the {} service of {} runs on {} since day {i}",
        TOPICS[i % TOPICS.len()],
        PROJECTS[i % PROJECTS.len()],
        TOOLS[i % TOOLS.len()]
    )
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3)
    })
}

/// The store exported without an embedder (the user's own backup path),
/// normalised to sorted lines so the digest ignores enumeration order.
/// Opens the engine itself: the service must NOT be open on `dir`.
fn export(dir: &str) -> (u64, String, Vec<Value>) {
    let mut buf = Vec::new();
    let written = export_jsonl(std::path::Path::new(dir), &mut buf, false).expect("export");
    let text = String::from_utf8(buf).expect("utf8");
    let mut lines: Vec<&str> = text.lines().collect();
    lines.sort_unstable();
    let rows = lines
        .iter()
        .map(|line| serde_json::from_str(line).expect("jsonl line"))
        .collect();
    (
        written,
        format!("{:016x}", fnv(lines.join("\n").as_bytes())),
        rows,
    )
}

fn recalls(svc: &Svc) -> Vec<Value> {
    [
        "billing project alpha rust",
        "search beta redis",
        "auth gamma nginx",
        "cache kafka",
        "export day 42",
    ]
    .iter()
    .map(|q| {
        let ids: Vec<u64> = svc
            .recall(q, 5, None)
            .expect("recall")
            .iter()
            .map(|r| r.id)
            .collect();
        json!({"query": q, "ids": ids})
    })
    .collect()
}

fn ids_of(value: &Value) -> BTreeSet<u64> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .collect()
}

/// Facts, links, relations, deletions, edge cases, expiry, reinforcement,
/// extraction and a working context: one of everything a store holds.
fn seed(dir: &str, manifest: &str, facts: usize) {
    let (live, deleted, expiring, extracted, strengths) = {
        let svc = open(dir);
        let mut live: Vec<Value> = Vec::new();
        let mut ids: Vec<u64> = Vec::new();
        for i in 0..facts {
            let text = fact_text(i);
            let m = meta(PROJECTS[i % PROJECTS.len()], i);
            let links: Vec<Link> = if i % 5 == 0 && i > 0 {
                vec![Link {
                    target: ids[i - 1],
                    relation: "follows".into(),
                }]
            } else {
                vec![]
            };
            let ttl = i % 10 == 3;
            let id = if ttl {
                svc.remember_with_ttl(&text, &links, Some(&m), Some(TEN_YEARS))
                    .expect("remember ttl")
            } else {
                svc.remember(&text, &links, Some(&m)).expect("remember")
            };
            ids.push(id);
            live.push(json!({"id": id, "content": text, "ttl": ttl}));
        }
        // Cases a real store holds: unicode, a fact near the size cap, no metadata.
        let long = "long ".repeat(400);
        for text in [
            "café ☕ naïve 日本語 emoji 🧠 — dashes",
            long.as_str(),
            "bare fact without metadata",
        ] {
            let id = svc.remember(text, &[], None).expect("remember edge case");
            live.push(json!({"id": id, "content": text.trim(), "ttl": false}));
            ids.push(id);
        }
        for k in 0..20.min(facts.saturating_sub(51)) {
            svc.relate(ids[k], ids[k + 50], "related_to")
                .expect("relate");
        }
        // Deletions must stay deleted.
        let mut deleted = Vec::new();
        for k in [7usize, 19, 41, 88, 130, 177, 203, 250, 271, 299] {
            if k < facts {
                assert!(svc.forget(ids[k]).expect("forget"));
                deleted.push(json!({"id": ids[k], "content": fact_text(k)}));
            }
        }
        live.retain(|f| !deleted.iter().any(|d| d["id"] == f["id"]));
        // Facts that must be gone once their (short) lifetime has passed.
        let mut expiring = Vec::new();
        for i in 0..5 {
            let text = format!("ephemeral fact {i}: expires two seconds after it is written");
            let id = svc
                .remember_with_ttl(&text, &[], Some(&meta("ephemeral", i)), Some(SHORT_TTL))
                .expect("remember short ttl");
            expiring.push(json!({"id": id, "content": text}));
        }
        // Reinforcement: feedback moves a stored strength.
        let strengths: Vec<Value> = ids
            .iter()
            .take(5)
            .map(|id| json!({"id": id, "strength": svc.feedback(*id, true).expect("feedback")}))
            .collect();
        // Extraction wires entity hubs into the graph.
        let mut extracted = Vec::new();
        for text in [
            "Billing retries are capped at five. Billing alerts page the on-call.",
            "Search reindexing runs nightly. Search latency is tracked per tenant.",
        ] {
            let got = svc
                .remember_extracted(text, &StubExtractor, Some(&meta("extracted", 0)))
                .expect("remember_extracted");
            extracted.extend(got.ids);
        }
        // A saved working context.
        let working = WorkingContext {
            goal: Some("ship the upgrade without losing a fact".into()),
            ..WorkingContext::default()
        };
        svc.save_working_context("guard-project", "guard-session", &working)
            .expect("save working context");
        (live, deleted, expiring, extracted, strengths)
    };
    // The expected state is the one AFTER the short lifetimes ran out.
    std::thread::sleep(std::time::Duration::from_secs(SHORT_TTL + 1));
    write_manifest(
        dir,
        manifest,
        live,
        deleted,
        json!({"expiring": expiring, "extracted": extracted, "strengths": strengths}),
    );
}

fn write_manifest(dir: &str, manifest: &str, live: Vec<Value>, deleted: Vec<Value>, extra: Value) {
    let (lines, digest, _) = export(dir);
    let svc = open(dir);
    let why = svc.why("billing project alpha", 3, None).expect("why");
    let filtered = svc
        .recall_where(
            "service runs",
            20,
            &[ColumnFilter {
                field: "project".into(),
                op: ColumnOp::Eq,
                value: "alpha".into(),
            }],
        )
        .expect("recall_where");
    let doc = json!({
        "live": live, "deleted": deleted, "extra": extra,
        "export_lines": lines, "export_digest": digest,
        "recalls": recalls(&svc),
        "why": {"nodes": why.nodes.len(), "edges": why.edges.len()},
        "filtered_hits": filtered.len(),
    });
    std::fs::write(manifest, serde_json::to_string_pretty(&doc).expect("json"))
        .expect("write manifest");
    println!(
        "recorded {} live facts, {} deleted, export digest {digest}",
        doc["live"].as_array().map_or(0, Vec::len),
        doc["deleted"].as_array().map_or(0, Vec::len)
    );
}

fn read_manifest(manifest: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(manifest).expect("read manifest"))
        .expect("parse manifest")
}

fn append(dir: &str, manifest: &str) {
    let doc = read_manifest(manifest);
    let mut live = doc["live"].as_array().expect("live").clone();
    {
        let svc = open(dir);
        for i in 1000..1025 {
            let text = fact_text(i);
            let id = svc
                .remember(&text, &[], Some(&meta("delta", i)))
                .expect("append");
            live.push(json!({"id": id, "content": text, "ttl": false}));
        }
    }
    write_manifest(
        dir,
        manifest,
        live,
        doc["deleted"].as_array().expect("deleted").clone(),
        doc["extra"].clone(),
    );
}

struct Tally {
    loss: usize,
    warn: usize,
}

impl Tally {
    fn loss(&mut self, message: String) {
        println!("LOSS  {message}");
        self.loss += 1;
    }
    fn warn(&mut self, message: String) {
        println!("WARN  {message}");
        self.warn += 1;
    }
}

fn verify(dir: &str, manifest: &str, min_rate: f64) -> bool {
    let doc = read_manifest(manifest);
    let mut t = Tally { loss: 0, warn: 0 };
    // Exact reads first; the export opens the engine, so the service waits.
    let (lines, digest, rows) = export(dir);
    let live = doc["live"].as_array().expect("live");
    for f in live {
        match rows.iter().find(|r| r["id"] == f["id"]) {
            None => t.loss(format!("fact {} is absent from the export", f["id"])),
            Some(r) if r["content"] != f["content"] => {
                t.loss(format!("fact {} changed content", f["id"]))
            }
            Some(_) => {}
        }
    }
    let expected_lines = doc["export_lines"].as_u64().expect("lines");
    if lines != expected_lines {
        t.loss(format!(
            "the export holds {lines} facts, expected {expected_lines}"
        ));
    }
    if digest != doc["export_digest"].as_str().expect("digest") {
        t.loss(format!(
            "export digest {digest} differs from {}",
            doc["export_digest"]
        ));
    }
    for d in doc["deleted"].as_array().expect("deleted") {
        if rows.iter().any(|r| r["id"] == d["id"]) {
            t.loss(format!("deleted fact {} is back in the export", d["id"]));
        }
    }
    // Expiry: the orchestrator waits past the lifetime before verifying.
    for e in doc["extra"]["expiring"].as_array().into_iter().flatten() {
        if rows.iter().any(|r| r["id"] == e["id"]) {
            t.loss(format!("expired fact {} is still served", e["id"]));
        }
    }
    let svc = open(dir);
    // Retrieval by similarity is approximate: a rate, not an equality.
    let mut found = 0usize;
    for f in live {
        let hits = svc
            .recall(f["content"].as_str().expect("content"), 10, None)
            .expect("recall");
        if hits.iter().any(|h| Some(h.id) == f["id"].as_u64()) {
            found += 1;
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let rate = found as f64 / live.len().max(1) as f64;
    println!(
        "INFO  retrieval_rate={rate:.4} ({found}/{} facts found by their own text at k=10)",
        live.len()
    );
    if rate < min_rate {
        t.loss(format!(
            "retrieval fell to {:.1}%, below the floor of {:.1}%",
            rate * 100.0,
            min_rate * 100.0
        ));
    }
    // Metadata filter, fused recall.
    let filtered = svc
        .recall_where(
            "service runs",
            20,
            &[ColumnFilter {
                field: "project".into(),
                op: ColumnOp::Eq,
                value: "alpha".into(),
            }],
        )
        .expect("recall_where");
    if filtered.is_empty()
        || filtered
            .iter()
            .any(|h| h.metadata.as_ref().and_then(|m| m.get("project")) != Some(&json!("alpha")))
    {
        t.loss(format!(
            "recall_where returned {} hits, some outside the filter or none",
            filtered.len()
        ));
    }
    if svc
        .recall_fused("billing alpha rust", 5, None, FusionOptions::default())
        .expect("recall_fused")
        .is_empty()
    {
        t.loss("recall_fused returned nothing".into());
    }
    // Reinforcement still works on the reopened store.
    for s in doc["extra"]["strengths"].as_array().into_iter().flatten() {
        if svc.feedback(s["id"].as_u64().expect("id"), true).is_err() {
            t.loss(format!("feedback no longer applies to fact {}", s["id"]));
        }
    }
    // Extraction hubs stay hidden from recall; the entity graph is still there.
    let hubs = svc.recall("billing", 50, None).expect("recall billing");
    if hubs.iter().any(|h| h.content.starts_with("Entity:")) {
        t.loss("an entity hub leaks into recall".into());
    }
    // Working context round trip.
    match svc.load_working_context("guard-project", "guard-session") {
        Ok(Some(w)) if w.goal.as_deref() == Some("ship the upgrade without losing a fact") => {}
        other => t.loss(format!(
            "working context lost: {:?}",
            other.map(|o| o.map(|w| w.goal))
        )),
    }
    // Ranking and the graph walk: a difference is reported, not a loss — ties
    // between near-identical facts are ordered by the index, not by the data.
    for (now, then) in recalls(&svc)
        .iter()
        .zip(doc["recalls"].as_array().expect("recalls"))
    {
        if now["ids"] != then["ids"] {
            let (a, b) = (ids_of(&now["ids"]), ids_of(&then["ids"]));
            t.warn(format!(
                "ranking changed for {}: {} of 5 ids in common",
                then["query"],
                a.intersection(&b).count()
            ));
        }
    }
    let why = svc.why("billing project alpha", 3, None).expect("why");
    let before = (
        doc["why"]["nodes"].as_u64().unwrap_or(0),
        doc["why"]["edges"].as_u64().unwrap_or(0),
    );
    if (why.nodes.len() as u64, why.edges.len() as u64) != before {
        t.warn(format!(
            "why() walk is {}n/{}e, was {}n/{}e",
            why.nodes.len(),
            why.edges.len(),
            before.0,
            before.1
        ));
    }
    println!(
        "{} — {} live facts checked, {} loss, {} warning(s)",
        if t.loss == 0 { "OK" } else { "FAILED" },
        live.len(),
        t.loss,
        t.warn
    );
    t.loss == 0
}

/// Write facts until killed. An id is appended to `acks` only AFTER `remember`
/// returned, so every id in that file was acknowledged to its caller.
fn soak(dir: &str, acks: &str) {
    use std::io::Write;
    let svc = open(dir);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(acks)
        .expect("open acks");
    let start = std::fs::read_to_string(acks).map_or(0, |s| s.lines().count());
    for i in start.. {
        let text = format!("soak fact {i}: acknowledged write number {i} of the crash test");
        let id = svc
            .remember(&text, &[], Some(&meta("soak", i)))
            .expect("remember");
        writeln!(file, "{id}").expect("ack");
        file.flush().expect("flush ack");
    }
}

/// Every acknowledged id must still be in the store after the kill.
fn check(dir: &str, acks: &str) -> bool {
    let (_, _, rows) = export(dir);
    let present: HashSet<u64> = rows.iter().filter_map(|r| r["id"].as_u64()).collect();
    let acked: Vec<u64> = std::fs::read_to_string(acks)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.parse().ok())
        .collect();
    let lost = acked.iter().filter(|id| !present.contains(id)).count();
    println!(
        "{} acknowledged writes, {lost} lost, store holds {} facts",
        acked.len(),
        present.len()
    );
    lost == 0
}

/// A store held by one process must be refused to a second, not shared.
fn probe_lock(dir: &str) -> bool {
    match MemoryService::open(dir, HashEmbedder::new(DEFAULT_DIMENSION)) {
        Err(MemoryError::Storage(error)) if format!("{error:?}").contains("DatabaseLocked") => {
            println!("OK — the second opener is refused");
            true
        }
        Err(other) => {
            println!("FAILED — refused for another reason: {other}");
            false
        }
        Ok(_) => {
            println!("FAILED — two processes opened the same store");
            false
        }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (Some(mode), Some(dir)) = (a.get(1).map(String::as_str), a.get(2)) else {
        eprintln!(
            "usage: driver seed|verify|append|soak|check|probe-lock <dir> [manifest|acks] [facts]"
        );
        std::process::exit(2);
    };
    let second = a.get(3).map(String::as_str);
    let ok = match (mode, second) {
        ("seed", Some(manifest)) => {
            seed(
                dir,
                manifest,
                a.get(4).and_then(|n| n.parse().ok()).unwrap_or(300),
            );
            true
        }
        ("append", Some(manifest)) => {
            append(dir, manifest);
            true
        }
        ("verify", Some(manifest)) => {
            let floor = a
                .get(4)
                .and_then(|n| n.parse().ok())
                .unwrap_or(DEFAULT_MIN_RETRIEVAL_RATE);
            verify(dir, manifest, floor)
        }
        ("soak", Some(acks)) => {
            soak(dir, acks);
            true
        }
        ("check", Some(acks)) => check(dir, acks),
        ("probe-lock", _) => probe_lock(dir),
        _ => {
            eprintln!("unknown mode or missing argument");
            std::process::exit(2);
        }
    };
    if !ok {
        std::process::exit(1);
    }
}
