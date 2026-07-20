//! Core logic for `velesdb-cli graph doctor`.
//!
//! Detects and (optionally) repairs legacy "phantom" edges: edges present
//! in a graph collection's edge store whose `source` or `target` node has
//! no stored payload. These can only exist via WAL/snapshot replay at
//! `Collection::open` — `add_edge`/`add_edges_batch` reject such edges
//! outright (#1442) — so `doctor` exists purely to audit/repair databases
//! created before that validation landed. Replay itself is intentionally
//! left unvalidated (see `docs/CONCURRENCY_MODEL.md`): filtering edges at
//! open time would silently drop data for legitimate edge-only graphs.
//!
//! `doctor` is a separate, explicit, opt-in tool — never part of the
//! replay path — and defaults to a read-only report (see #1469).

use std::collections::{BTreeSet, HashMap};

use colored::Colorize;
use velesdb_core::GraphCollection;

use crate::helpers;

/// Number of phantom edges shown in the printed sample (table and JSON).
const SAMPLE_SIZE: usize = 10;

/// A single phantom edge: present in the edge store, missing one or both
/// endpoint payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PhantomEdge {
    pub edge_id: u64,
    pub source: u64,
    pub target: u64,
    pub label: String,
    pub missing_source: bool,
    pub missing_target: bool,
}

/// Which repair action `doctor` should take, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DoctorMode {
    /// Report-only: no mutation (the default).
    Report,
    /// Remove phantom edges from the edge store.
    Purge,
    /// Seed a minimal `{}` payload for each missing endpoint.
    Stub,
}

impl DoctorMode {
    fn label(self) -> &'static str {
        match self {
            DoctorMode::Report => "report",
            DoctorMode::Purge => "purge",
            DoctorMode::Stub => "stub",
        }
    }
}

/// Returns whether `node_id` has a stored payload, memoizing lookups so a
/// high-degree node is only queried once per scan.
fn node_has_payload(
    col: &GraphCollection,
    cache: &mut HashMap<u64, bool>,
    node_id: u64,
) -> anyhow::Result<bool> {
    if let Some(&cached) = cache.get(&node_id) {
        return Ok(cached);
    }
    let has = col
        .get_node_payload(node_id)
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .is_some();
    cache.insert(node_id, has);
    Ok(has)
}

/// Scans every edge in `col` and returns those whose source and/or target
/// has no stored payload (legacy phantom edges).
///
/// # Errors
///
/// Returns an error if a node payload lookup fails.
pub(crate) fn scan_phantom_edges(col: &GraphCollection) -> anyhow::Result<Vec<PhantomEdge>> {
    let edges = col.get_edges(None);
    let mut cache = HashMap::new();
    let mut phantoms = Vec::new();
    for edge in &edges {
        let missing_source = !node_has_payload(col, &mut cache, edge.source())?;
        let missing_target = !node_has_payload(col, &mut cache, edge.target())?;
        if missing_source || missing_target {
            phantoms.push(PhantomEdge {
                edge_id: edge.id(),
                source: edge.source(),
                target: edge.target(),
                label: edge.label().to_string(),
                missing_source,
                missing_target,
            });
        }
    }
    Ok(phantoms)
}

/// Removes every phantom edge from the edge store. Returns the number
/// actually removed. Naturally idempotent: a re-scan after purging finds
/// no phantoms, so a second call receives an empty slice.
pub(crate) fn purge_phantom_edges(col: &GraphCollection, phantoms: &[PhantomEdge]) -> usize {
    phantoms
        .iter()
        .filter(|p| col.remove_edge(p.edge_id))
        .count()
}

/// Seeds a minimal `{}` payload for every node referenced as a missing
/// endpoint. Returns the number of distinct nodes stubbed. Naturally
/// idempotent: once a node has a payload it no longer appears as missing
/// on a re-scan.
///
/// # Errors
///
/// Returns an error if writing a stub payload fails.
pub(crate) fn stub_phantom_edges(
    col: &GraphCollection,
    phantoms: &[PhantomEdge],
) -> anyhow::Result<usize> {
    let mut missing_nodes: BTreeSet<u64> = BTreeSet::new();
    for p in phantoms {
        if p.missing_source {
            missing_nodes.insert(p.source);
        }
        if p.missing_target {
            missing_nodes.insert(p.target);
        }
    }
    for &node_id in &missing_nodes {
        col.upsert_node_payload(node_id, &serde_json::json!({}))
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    Ok(missing_nodes.len())
}

fn phantom_to_json(p: &PhantomEdge) -> serde_json::Value {
    serde_json::json!({
        "edge_id": p.edge_id,
        "source": p.source,
        "target": p.target,
        "label": p.label,
        "missing_source": p.missing_source,
        "missing_target": p.missing_target,
    })
}

/// Prints the doctor report (phantom count, sample, action taken).
///
/// # Errors
///
/// Returns an error if JSON serialization fails.
pub(crate) fn print_report(
    collection: &str,
    phantoms: &[PhantomEdge],
    mode: DoctorMode,
    fixed: usize,
    format: &str,
) -> anyhow::Result<()> {
    if format == "json" {
        let sample: Vec<_> = phantoms
            .iter()
            .take(SAMPLE_SIZE)
            .map(phantom_to_json)
            .collect();
        helpers::print_json(&serde_json::json!({
            "collection": collection,
            "phantom_edge_count": phantoms.len(),
            "mode": mode.label(),
            "fixed": fixed,
            "sample": sample,
        }))?;
    } else {
        print_report_table(collection, phantoms, mode, fixed);
    }
    Ok(())
}

fn print_report_table(collection: &str, phantoms: &[PhantomEdge], mode: DoctorMode, fixed: usize) {
    println!(
        "\n{} '{}'\n",
        "Graph Doctor".bold().underline(),
        collection.green()
    );
    if phantoms.is_empty() {
        println!("  {} No phantom edges found.\n", "✅".green());
        return;
    }
    println!(
        "  {} {} phantom edge(s) found (edge store references a node with no stored payload)\n",
        "⚠️".yellow(),
        phantoms.len().to_string().yellow()
    );
    for p in phantoms.iter().take(SAMPLE_SIZE) {
        let missing = match (p.missing_source, p.missing_target) {
            (true, true) => "source+target".to_string(),
            (true, false) => format!("source={}", p.source),
            (false, true) => format!("target={}", p.target),
            (false, false) => String::new(),
        };
        println!(
            "  {} {} --[{}]--> {}  missing={}",
            format!("[{}]", p.edge_id).cyan(),
            p.source,
            p.label.green(),
            p.target,
            missing.red(),
        );
    }
    if phantoms.len() > SAMPLE_SIZE {
        println!("  ... and {} more", phantoms.len() - SAMPLE_SIZE);
    }
    match mode {
        DoctorMode::Report => {
            println!(
                "\n  {} Dry-run: no changes made. Re-run with --purge or --stub to repair.\n",
                "ℹ️".cyan()
            );
        }
        DoctorMode::Purge => {
            println!(
                "\n  {} {} phantom edge(s) removed.\n",
                "✅".green(),
                fixed.to_string().green()
            );
        }
        DoctorMode::Stub => {
            println!(
                "\n  {} {} missing node(s) stubbed with an empty payload.\n",
                "✅".green(),
                fixed.to_string().green()
            );
        }
    }
}
