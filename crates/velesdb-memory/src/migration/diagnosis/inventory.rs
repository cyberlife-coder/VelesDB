use super::{Capability, CollectionInventory, SourceProvenance, TtlSummary};
use crate::migration::enumeration::{enumerate_by_cursor, scroll_page, AGENT_COLLECTIONS};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

/// The batch size the inventory walks with — large enough that the walk is not
/// dominated by per-batch overhead, small enough that a store far bigger than
/// memory is still read in bounded chunks.
const INVENTORY_BATCH: usize = 1024;

pub(super) struct StoreInventory {
    pub(super) source_provenance: SourceProvenance,
    pub(super) source_dimension: Option<usize>,
    pub(super) collections: Vec<CollectionInventory>,
    pub(super) edge_counts: Capability,
    pub(super) facts: u64,
    pub(super) edges: u64,
    pub(super) working_contexts: u64,
    pub(super) reserved_metadata: BTreeSet<String>,
    pub(super) ttl: TtlSummary,
}

pub(super) fn inspect(source: &Path) -> Result<StoreInventory, crate::MemoryError> {
    let source_provenance = read_provenance(source);
    let db = Arc::new(velesdb_core::Database::open(source)?);
    let mut collections: Vec<CollectionInventory> = AGENT_COLLECTIONS
        .iter()
        .map(|name| inventory_collection(&db, name))
        .collect::<Result<_, _>>()?;
    let source_dimension = agreed_dimension(&collections);
    let edge_counts = attach_edge_counts(&db, &mut collections, source_dimension);
    let totals = totals(&collections);

    Ok(StoreInventory {
        source_provenance,
        source_dimension,
        collections,
        edge_counts,
        facts: totals.facts,
        edges: totals.edges,
        working_contexts: totals.working_contexts,
        reserved_metadata: totals.reserved_metadata,
        ttl: totals.ttl,
    })
}

/// Store-wide sums, folded one collection at a time.
#[derive(Default)]
struct Totals {
    facts: u64,
    edges: u64,
    working_contexts: u64,
    reserved_metadata: BTreeSet<String>,
    ttl: TtlSummary,
}

fn totals(collections: &[CollectionInventory]) -> Totals {
    let mut totals = Totals::default();
    for inventory in collections {
        totals.fold(inventory);
    }
    totals
}

impl Totals {
    fn fold(&mut self, inventory: &CollectionInventory) {
        self.facts += inventory.facts;
        self.edges += inventory.edges.unwrap_or(0);
        self.working_contexts += inventory.working_contexts;
        self.reserved_metadata
            .extend(inventory.reserved_metadata.iter().cloned());
        self.ttl.merge(&inventory.ttl);
    }
}

/// What the store records about its embedder, phrased so an absent record
/// cannot be misread as a match.
fn read_provenance(source: &Path) -> SourceProvenance {
    match crate::embedding_provenance::read(source) {
        Ok(Some(provenance)) => SourceProvenance::Known {
            model: provenance.model,
            dimension: provenance.dimension,
        },
        Ok(None) => SourceProvenance::Unknown {
            reason: format!(
                "no {} in the store: it predates embedding-model recording, so the model that \
                 filled it is not knowable from disk. Only the vector WIDTH can be compared, and \
                 two different models of the same width are indistinguishable here.",
                crate::embedding_provenance::PROVENANCE_FILE
            ),
        },
        Err(err) => SourceProvenance::Unknown {
            reason: format!(
                "the embedding record exists but could not be read ({err}) — which is not the \
                 same as absent, and is reported as unknown rather than as a match."
            ),
        },
    }
}

/// Walk one collection and describe it.
///
/// The counts come from the WALK, not from `point_count`: the config's count is
/// what the collection believes, and the rebuild will carry what the walk
/// actually yields. Where those two disagree, the walk is the one that matters.
fn inventory_collection(
    db: &velesdb_core::Database,
    name: &str,
) -> Result<CollectionInventory, crate::MemoryError> {
    let Some(any) = db.get_any_collection(name) else {
        return Ok(CollectionInventory::absent(name));
    };
    let mut inventory = CollectionInventory {
        name: name.to_owned(),
        present: true,
        dimension: Some(any.config().dimension),
        facts: 0,
        edges: None,
        working_contexts: 0,
        reserved_metadata: BTreeSet::new(),
        ttl: TtlSummary::default(),
    };
    let mut cursor: Option<u64> = None;
    loop {
        let (facts, next) = scroll_page(db, name, cursor, INVENTORY_BATCH)?;
        if facts.is_empty() {
            break;
        }
        for fact in &facts {
            fold_payload(&mut inventory, &fact.payload);
        }
        match next {
            Some(next_cursor) => cursor = Some(next_cursor),
            None => break,
        }
    }
    Ok(inventory)
}

/// Fold one stored payload into the collection's tallies.
fn fold_payload(inventory: &mut CollectionInventory, payload: &str) {
    inventory.facts += 1;
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(payload) else {
        return;
    };
    for key in map.keys() {
        if crate::storage::is_reserved_key(key) {
            inventory.reserved_metadata.insert(key.clone());
        }
    }
    if map.contains_key(crate::storage::CTX_WORKING_FIELD) {
        inventory.working_contexts += 1;
    }
    if let Some(expiry) = map
        .get(velesdb_core::collection::EXPIRES_AT_KEY)
        .and_then(Value::as_u64)
    {
        inventory.ttl.observe(expiry);
    }
}

/// The one width every present collection shares, or `None` when they disagree.
///
/// They disagreeing is not a theoretical case to shrug at: `AgentMemory` opens
/// all three at ONE dimension, so a store whose collections drifted apart
/// cannot be opened by it at all, and the rebuild has no single source width to
/// read from.
fn agreed_dimension(collections: &[CollectionInventory]) -> Option<usize> {
    let mut dimensions = collections
        .iter()
        .filter_map(|collection| collection.dimension);
    let first = dimensions.next()?;
    dimensions
        .all(|dimension| dimension == first)
        .then_some(first)
}

/// Count the edges of each collection, when doing so is safe.
///
/// Edges are reachable from outside `velesdb-core` only through `AgentMemory`,
/// because the three agent collections are created as VECTOR collections while
/// the edge API is published on `GraphCollection` — so `as_graph()` returns
/// `None` on exactly these three and the graph route is closed.
///
/// `AgentMemory` is therefore the only route, and it comes with a hazard worth
/// stating: constructing it CREATES any collection it does not find. On a
/// complete store that is a no-op, but on a store missing one of the three it
/// would write — during a diagnosis whose whole contract is that it does not.
/// So the counts are taken only when all three are present at one agreed width,
/// and the capability records why when they are not.
fn attach_edge_counts(
    db: &Arc<velesdb_core::Database>,
    collections: &mut [CollectionInventory],
    source_dimension: Option<usize>,
) -> Capability {
    if !collections.iter().all(|collection| collection.present) {
        return Capability::Missing {
            blocker: "at least one of the three agent collections is absent, and constructing \
                      `AgentMemory` to reach the edge API would CREATE it — a write a diagnosis \
                      must not perform. Edge counts are not established for this store."
                .to_owned(),
        };
    }
    let Some(dimension) = source_dimension else {
        return Capability::Missing {
            blocker: "the collections do not share one width, so `AgentMemory` — which opens all \
                      three at a single dimension — cannot be constructed to reach the edge API."
                .to_owned(),
        };
    };
    match count_edges(db, collections, dimension) {
        Ok(total) => Capability::Proven {
            evidence: format!(
                "walked every id of the three collections through `AgentMemory::relations` at the \
                 source width {dimension} and summed the outgoing edges: {total}. Outgoing-only \
                 is what makes each edge count once — every edge has exactly one source."
            ),
        },
        Err(err) => Capability::Missing {
            blocker: format!("the edge walk failed: {err}"),
        },
    }
}

/// Sum each collection's outgoing edges through `AgentMemory`, writing the
/// per-collection count back into the inventory.
fn count_edges(
    db: &Arc<velesdb_core::Database>,
    collections: &mut [CollectionInventory],
    dimension: usize,
) -> Result<u64, crate::MemoryError> {
    let memory = velesdb_core::agent::AgentMemory::with_dimension(Arc::clone(db), dimension)?;
    let mut total = 0u64;
    for inventory in collections.iter_mut() {
        let ids: Vec<u64> = enumerate_by_cursor(db, &inventory.name, INVENTORY_BATCH)?
            .into_iter()
            .map(|fact| fact.id)
            .collect();
        let mut count = 0u64;
        for id in ids {
            count += edges_of(&memory, &inventory.name, id)?;
        }
        inventory.edges = Some(count);
        total += count;
    }
    Ok(total)
}

/// Outgoing edges of one fact, dispatched on which subsystem owns it.
fn edges_of(
    memory: &velesdb_core::agent::AgentMemory,
    collection: &str,
    id: u64,
) -> Result<u64, crate::MemoryError> {
    let edges = match collection {
        "_semantic_memory" => memory.semantic().relations(id)?,
        "_episodic_memory" => memory.episodic().relations(id)?,
        "_procedural_memory" => memory.procedural().relations(id)?,
        other => {
            return Err(velesdb_core::Error::Query(format!(
                "`{other}` is not one of the agent collections"
            ))
            .into())
        }
    };
    Ok(u64::try_from(edges.len()).unwrap_or(u64::MAX))
}
