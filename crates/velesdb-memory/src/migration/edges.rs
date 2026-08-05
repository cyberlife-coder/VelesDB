//! Lossless edge export and reinsertion (#1762, PR C2a).
//!
//! The fact export ([`super::enumeration`]) moves points. It cannot move
//! relations, and a rebuild that shipped without them would hand back a store
//! whose facts are all present and whose graph is empty — a loss that no
//! per-fact comparison detects, because every fact is intact.
//!
//! # Why there is no transport type here
//!
//! Facts travel as [`RawFact`](super::RawFact), a type this module's sibling
//! defines. Edges travel as [`GraphEdge`] itself, the engine's own type, and
//! that is deliberate. `velesdb-memory` already owns a reduced edge — the
//! public `MemoryEdge` (`crate::model`) — and it has no `properties` field at
//! all: `storage::to_memory_edges` builds it from `id`, `source`, `target` and
//! `label` and never calls `edge.properties()`. Reusing it here would have
//! compiled, round-tripped, and lost every property in silence. Carrying the
//! engine's own tuple means there is no field for a conversion to forget.
//!
//! # What makes the export sound
//!
//! An edge is exported when it lies between two facts the fact export also
//! carries. That is not a filter this module implements: the edge walk and the
//! fact walk already agree on which endpoints are live, because
//! `MemoryTtl`'s map is refilled from the durable `_veles_expires_at` key by
//! every subsystem constructor (`rebuild_ttl_from_payloads`). The agreement is
//! MEASURED, three-armed, in `tests::edges::the_two_walks_agree_on_which_endpoints_are_live`
//! — it was reasoned wrongly twice before it was measured once.
//!
//! # The one shape this refuses
//!
//! Every edge `relate` writes derives its id from its triple through
//! [`velesdb_core::hash_edge_id`]. `VelesQL` DML does not: it accepts an
//! explicit edge id. An edge whose id its triple does not derive cannot be
//! reinserted honestly — `relate` at the destination would compute a
//! *different* id, and one logical edge would end up with two identities across
//! the migration. Rather than renumber it silently, the export stops and names
//! it.

use velesdb_core::agent::AgentMemory;
use velesdb_core::collection::graph::GraphEdge;
use velesdb_core::Database;

use super::enumeration::{enumerate_by_cursor, AGENT_COLLECTIONS};

/// Which agent subsystem owns a collection.
///
/// The three expose byte-identical `relations`/`incoming_relations`/`relate`
/// signatures but are distinct types, so the dispatch is explicit rather than
/// generic. `tests::edges::every_agent_collection_is_dispatchable` is what
/// proves this list has not drifted from [`AGENT_COLLECTIONS`].
#[derive(Debug, Clone, Copy)]
enum Subsystem {
    Semantic,
    Episodic,
    Procedural,
}

fn subsystem_of(collection: &str) -> Result<Subsystem, crate::MemoryError> {
    if collection == AGENT_COLLECTIONS[0] {
        Ok(Subsystem::Semantic)
    } else if collection == AGENT_COLLECTIONS[1] {
        Ok(Subsystem::Episodic)
    } else if collection == AGENT_COLLECTIONS[2] {
        Ok(Subsystem::Procedural)
    } else {
        Err(velesdb_core::Error::Query(format!(
            "`{collection}` is not an agent memory collection; edges are exported \
             per subsystem, and the subsystems are {AGENT_COLLECTIONS:?}"
        ))
        .into())
    }
}

/// Which adjacency map a walk reads.
///
/// `outgoing` and `incoming` are separate maps, so an edge missing from one of
/// them is caught by the other. They are not separate copies: both resolve the
/// id through the same stored record. See [`cross_check_edges`] for what that
/// buys and what it does not.
#[derive(Debug, Clone, Copy)]
enum Direction {
    Outgoing,
    Incoming,
}

fn edges_at(
    memory: &AgentMemory,
    subsystem: Subsystem,
    direction: Direction,
    id: u64,
) -> Result<Vec<GraphEdge>, crate::MemoryError> {
    let result = match (subsystem, direction) {
        (Subsystem::Semantic, Direction::Outgoing) => memory.semantic().relations(id),
        (Subsystem::Semantic, Direction::Incoming) => memory.semantic().incoming_relations(id),
        (Subsystem::Episodic, Direction::Outgoing) => memory.episodic().relations(id),
        (Subsystem::Episodic, Direction::Incoming) => memory.episodic().incoming_relations(id),
        (Subsystem::Procedural, Direction::Outgoing) => memory.procedural().relations(id),
        (Subsystem::Procedural, Direction::Incoming) => memory.procedural().incoming_relations(id),
    };
    result.map_err(crate::MemoryError::from)
}

/// Refuse an edge whose id its own triple does not derive.
///
/// Returning the edge rather than `()` keeps the guard on the value path, so a
/// caller cannot collect the edge and forget to call this.
fn require_derived_id(edge: GraphEdge) -> Result<GraphEdge, crate::MemoryError> {
    let derived = velesdb_core::hash_edge_id(edge.source(), edge.target(), edge.label());
    if edge.id() == derived {
        return Ok(edge);
    }
    Err(velesdb_core::Error::Query(format!(
        "edge {} ({} -{}-> {}) carries an id its triple does not derive (expected \
         {derived}); reinserting it would rederive the expected id and give one \
         logical edge two identities, so the export stops here rather than \
         renumbering it",
        edge.id(),
        edge.source(),
        edge.label(),
        edge.target(),
    ))
    .into())
}

/// Collect the edges reachable from `ids` through `direction`.
///
/// Takes the fact ids rather than finding them, so that a caller checking one
/// direction against the other can hand both walks the SAME snapshot. That is
/// not a convenience: expiry is evaluated against the wall clock on every read,
/// so two walks that each enumerated the store afresh could straddle a fact's
/// expiry instant and disagree — reporting a divergence that is a tick of the
/// clock rather than a lost edge, and aborting a migration whose indexes were
/// never anything but consistent.
fn collect(
    memory: &AgentMemory,
    subsystem: Subsystem,
    ids: &[u64],
    direction: Direction,
) -> Result<Vec<GraphEdge>, crate::MemoryError> {
    let mut out: Vec<GraphEdge> = Vec::new();
    let mut seen: std::collections::HashSet<u64> = std::collections::HashSet::new();
    for &id in ids {
        for edge in edges_at(memory, subsystem, direction, id)? {
            let edge = require_derived_id(edge)?;
            // Each walk reads ONE index, and an edge id is unique within a
            // collection — the edge store refuses a duplicate id inside its
            // write guard. A self-loop is pushed into both the outgoing and the
            // incoming index, but a single walk still meets it once. So this
            // branch is unreachable today, measured rather than assumed, and it
            // is an ERROR rather than a silent skip: were it ever reached, the
            // edge store would be handing out one id twice and quietly keeping
            // the first is the worst available answer.
            if !seen.insert(edge.id()) {
                return Err(velesdb_core::Error::Query(format!(
                    "edge {} was reported twice by a single {direction:?} walk of \
                     one collection; edge ids are unique per collection, so the \
                     edge index is inconsistent and no export from it can be \
                     trusted",
                    edge.id(),
                ))
                .into());
            }
            out.push(edge);
        }
    }
    Ok(out)
}

fn live_ids(db: &Database, collection: &str, batch: usize) -> Result<Vec<u64>, crate::MemoryError> {
    Ok(enumerate_by_cursor(db, collection, batch)?
        .into_iter()
        .map(|fact| fact.id)
        .collect())
}

/// Every edge of `collection` that lies between two exported facts, as complete
/// tuples.
///
/// Walks the live fact ids with the same cursor the fact export uses, then
/// takes each fact's outgoing edges. Each edge is checked against
/// [`require_derived_id`] before it is collected.
///
/// # Errors
/// Returns [`crate::MemoryError`] if `collection` is not an agent subsystem, if
/// the fact walk fails, or if any edge carries an id its triple does not derive.
pub fn export_edges(
    memory: &AgentMemory,
    db: &Database,
    collection: &str,
    batch: usize,
) -> Result<Vec<GraphEdge>, crate::MemoryError> {
    let subsystem = subsystem_of(collection)?;
    collect(
        memory,
        subsystem,
        &live_ids(db, collection, batch)?,
        Direction::Outgoing,
    )
}

/// The export, checked against the incoming index over ONE snapshot of the live
/// facts.
///
/// This is the entry point a rebuild should use. [`export_edges`] and
/// [`cross_check_edges`] each enumerate the store for themselves, which is fine
/// in isolation and wrong as a pair: expiry is evaluated against the wall clock
/// on every read, so a fact whose expiry falls between the two calls leaves the
/// two walks disagreeing about an edge neither of them lost. Enumerating once
/// and walking both directions over that one list removes the window rather than
/// narrowing it.
///
/// # Errors
/// Returns [`crate::MemoryError`] under the conditions of [`export_edges`], and
/// additionally when the two indexes do not yield the same set of tuples.
pub fn export_edges_verified(
    memory: &AgentMemory,
    db: &Database,
    collection: &str,
    batch: usize,
) -> Result<Vec<GraphEdge>, crate::MemoryError> {
    let subsystem = subsystem_of(collection)?;
    let ids = live_ids(db, collection, batch)?;
    let exported = collect(memory, subsystem, &ids, Direction::Outgoing)?;
    let crossed = collect(memory, subsystem, &ids, Direction::Incoming)?;
    let (left, right) = (comparable(&exported), comparable(&crossed));
    if left != right {
        return Err(velesdb_core::Error::Query(format!(
            "the outgoing and incoming edge indexes disagree over the same {} live \
             facts: {} tuples out, {} tuples in, {} present in only one of them. \
             An export cannot be called lossless while the two indexes describe \
             different graphs",
            ids.len(),
            left.len(),
            right.len(),
            left.symmetric_difference(&right).count(),
        ))
        .into());
    }
    Ok(exported)
}

/// An edge reduced to something orderable, so two collections of edges compare
/// as SETS OF TUPLES. Comparing counts would let two walks that had each lost
/// one edge agree.
fn comparable(edges: &[GraphEdge]) -> std::collections::BTreeSet<(u64, u64, u64, String, String)> {
    edges
        .iter()
        .map(|edge| {
            let properties: std::collections::BTreeMap<_, _> = edge.properties().iter().collect();
            (
                edge.id(),
                edge.source(),
                edge.target(),
                edge.label().to_owned(),
                serde_json::to_string(&properties).unwrap_or_default(),
            )
        })
        .collect()
}

/// The same collection of edges, gathered through the INCOMING index instead.
///
/// Prefer [`export_edges_verified`], which walks both directions over one
/// snapshot; this is the single-direction primitive, and comparing its result
/// against a separately-enumerated [`export_edges`] reintroduces the clock
/// window that entry point exists to close.
///
/// # What the agreement of the two indexes does and does not prove
///
/// `outgoing` and `incoming` are separate adjacency maps, so an edge that fell
/// out of one of them is caught. They are NOT separate copies of the edge: both
/// resolve the id through the same `edges` map in the shard, so a tuple whose
/// stored properties were corrupted would come back identically corrupted
/// through both. This checks MEMBERSHIP in two indexes, not the content twice.
/// The content is checked by re-reading the destination after reinsertion.
///
/// # Errors
/// Returns [`crate::MemoryError`] under the same conditions as [`export_edges`].
pub fn cross_check_edges(
    memory: &AgentMemory,
    db: &Database,
    collection: &str,
    batch: usize,
) -> Result<Vec<GraphEdge>, crate::MemoryError> {
    let subsystem = subsystem_of(collection)?;
    collect(
        memory,
        subsystem,
        &live_ids(db, collection, batch)?,
        Direction::Incoming,
    )
}

/// What putting the edges back produced.
///
/// Deliberately thin. It is NOT the verdict, and a caller that treats it as one
/// has been misled: `relate` is idempotent on an id that already exists and
/// IGNORES the properties it was handed in that case, so a destination that
/// dropped every property would still report every edge inserted. The verdict
/// is a re-read of the destination compared against the export — see
/// `tests::edges::reinserted_edges_are_read_back_identical_at_the_destination`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct EdgeReinsertion {
    /// Edges `relate` accepted, each having answered with the exported id.
    pub inserted: u64,
}

/// Put `edges` back into `collection`, AFTER the facts.
///
/// `relate` requires both endpoints live, so this cannot run before the fact
/// reinsertion — not as a matter of tidiness but because it would fail. The id
/// `relate` answers with is compared against the exported id on every edge: they
/// must agree, since both sides derive it from the same triple, and a
/// disagreement means the destination is deriving ids differently from the
/// source and the whole export is void.
///
/// # Errors
/// Returns [`crate::MemoryError`] if `collection` is not an agent subsystem, if
/// an endpoint is missing or expired at the destination, or if a reinserted edge
/// answers with an id other than the one exported.
pub fn reinsert_edges(
    memory: &AgentMemory,
    collection: &str,
    edges: &[GraphEdge],
) -> Result<EdgeReinsertion, crate::MemoryError> {
    let subsystem = subsystem_of(collection)?;
    let mut inserted = 0u64;
    for edge in edges {
        let properties: serde_json::Map<String, serde_json::Value> = edge
            .properties()
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        let properties = if properties.is_empty() {
            None
        } else {
            Some(properties)
        };
        let returned = relate_on(
            memory,
            subsystem,
            (edge.source(), edge.target()),
            edge.label(),
            properties.as_ref(),
        )?;
        if returned != edge.id() {
            return Err(velesdb_core::Error::Query(format!(
                "edge {} ({} -{}-> {}) was reinserted under id {returned}; the \
                 destination derives edge ids differently from the source, so no \
                 edge in this export can be trusted to keep its identity",
                edge.id(),
                edge.source(),
                edge.label(),
                edge.target(),
            ))
            .into());
        }
        inserted += 1;
    }
    Ok(EdgeReinsertion { inserted })
}

fn relate_on(
    memory: &AgentMemory,
    subsystem: Subsystem,
    endpoints: (u64, u64),
    label: &str,
    properties: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Result<u64, crate::MemoryError> {
    let (from, to) = endpoints;
    let result = match subsystem {
        Subsystem::Semantic => memory.semantic().relate(from, to, label, properties),
        Subsystem::Episodic => memory.episodic().relate(from, to, label, properties),
        Subsystem::Procedural => memory.procedural().relate(from, to, label, properties),
    };
    result.map_err(crate::MemoryError::from)
}
