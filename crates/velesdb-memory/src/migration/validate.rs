//! Validation of the rebuilt destination (#1762, PR C3).
//!
//! The rebuild re-reads what it writes, but it does so collection by
//! collection, WHILE writing. This is the other proof: one pass over the
//! finished destination against the source as it stands, before anything is
//! allowed to move. [`Phase::DestinationValidated`] — defined since C1 and
//! never produced by any code until now — is exactly this pass's journal
//! entry.
//!
//! # What is compared, and what deliberately is not
//!
//! Facts are compared as id → payload maps, both sides walked by the same
//! cursor the rebuild used; edges as sets of complete tuples through
//! [`super::edges::export_edges_verified`] on both stores. Vectors are NOT
//! compared: under `reembed` they differ from the source's by design, and
//! re-embedding every fact to check them would repeat the rebuild to validate
//! the rebuild. What stands in for them is the embedder witness the journal
//! already carries.
//!
//! # The one tolerated divergence
//!
//! A fact whose ABSOLUTE expiry passes between the source walk and the
//! destination walk is visible in one and hidden in the other, with nothing
//! lost — the clock window named in C2b's review, discriminated mechanically
//! here as promised there: a diverging id explains itself if and only if the
//! point's payload carries `_veles_expires_at <= now`. A durably expired fact
//! is invisible to BOTH walks, so this discriminator can never excuse real
//! loss: a live intruder or a missing live fact has no expiry to hide behind.
//!
//! # The provenance stamp
//!
//! The daemon reads `embedding-provenance.json` BEFORE it opens a store. A
//! destination switched live without a stamp would degrade into the
//! unrecorded-model warning on every start, and stamping it any earlier than
//! validation would stamp a store nobody had proven. The stamp is written from
//! the JOURNAL's identity — the same model and dimension every resume was
//! checked against — after the comparison passes and before the phase
//! advances.

use std::collections::BTreeMap;
use std::path::Path;

use velesdb_core::Database;

use super::diagnosis::TargetContract;
use super::edges::export_edges_verified;
use super::enumeration::{enumerate_by_cursor, AGENT_COLLECTIONS};
use super::execute::journal_workspace;
use super::state::{CollectionProgress, MigrationLock, MigrationState, Phase};
use velesdb_core::agent::AgentMemory;
use velesdb_core::collection::graph::GraphEdge;

/// What one validation pass established.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct ValidationOutcome {
    /// Facts compared across the two stores.
    pub facts: u64,
    /// Edges compared across the two stores.
    pub edges: u64,
    /// Divergences explained by an absolute expiry crossing the walk window —
    /// tolerated, counted, and reported rather than silently absorbed.
    pub explained_by_expiry: u64,
}

/// Validate `destination` against `store` and journal the result.
///
/// Requires the journal at [`Phase::Prepared`] with every collection
/// `Complete` (or already at [`Phase::DestinationValidated`], making the call
/// an idempotent re-validation). The source must still match the journalled
/// fingerprint: a source that moved since the rebuild invalidates the
/// comparison, not the destination, and the refusal says which.
///
/// # Errors
/// Returns [`crate::MemoryError`] when there is no journal, the rebuild is
/// unfinished, the source changed, the identity mismatches, the stores
/// diverge beyond the expiry window, or the stamp or journal write fails.
pub fn validate_destination(
    store: &Path,
    destination: &Path,
    target: &TargetContract,
    batch: usize,
) -> Result<ValidationOutcome, crate::MemoryError> {
    let workspace = journal_workspace(destination)?;
    let lock = MigrationLock::acquire(&workspace, "migrate-validate").map_err(query_error)?;
    let result = validate_locked(store, destination, target, batch, &workspace, &lock);
    let released = lock.release().map_err(query_error);
    let outcome = result?;
    released?;
    Ok(outcome)
}

fn validate_locked(
    store: &Path,
    destination: &Path,
    target: &TargetContract,
    batch: usize,
    workspace: &Path,
    lock: &MigrationLock,
) -> Result<ValidationOutcome, crate::MemoryError> {
    let mut state = journalled_state(target, workspace)?;
    let outcome = compare_stores(store, destination, &state, batch)?;

    crate::embedding_provenance::write(
        destination,
        &crate::embedding_provenance::EmbeddingProvenance::new(
            &state.target_model,
            state.target_dimension,
        ),
    )
    .map_err(query_error)?;

    if state.phase == Phase::Prepared {
        state.phase = Phase::DestinationValidated;
        state.write(workspace, lock).map_err(query_error)?;
    }
    Ok(outcome)
}

/// Open both stores and compare every collection's facts and edges.
fn compare_stores(
    store: &Path,
    destination: &Path,
    state: &MigrationState,
    batch: usize,
) -> Result<ValidationOutcome, crate::MemoryError> {
    let source = StoreView::open_source(store, state.target_dimension)?;
    let destination = StoreView::open_destination(destination, state.target_dimension)?;
    let mut outcome = ValidationOutcome::default();
    for collection in AGENT_COLLECTIONS {
        Comparison {
            source: &source,
            destination: &destination,
            collection,
            batch,
            outcome: &mut outcome,
        }
        .run()?;
    }
    Ok(outcome)
}

/// Read the journal and refuse everything a validation cannot stand on.
fn journalled_state(
    target: &TargetContract,
    workspace: &Path,
) -> Result<MigrationState, crate::MemoryError> {
    let state = MigrationState::read(workspace)
        .map_err(query_error)?
        .ok_or_else(|| {
            query_error(format!(
                "no migration journal at {}; there is nothing to validate — run \
                 the rebuild first",
                workspace.display()
            ))
        })?;
    require_validatable(&state)?;
    let fingerprint = super::filesystem::fingerprint(&state.source_path)?;
    state
        .may_resume(
            &state.source_path,
            &fingerprint,
            &target.model,
            target.dimension,
        )
        .map_err(|reason| {
            query_error(format!(
                "the comparison would be against a store the destination was \
                 not built from: {reason}"
            ))
        })?;
    Ok(state)
}

/// The journal must stand where a validation makes sense: rebuild finished,
/// switch not yet begun.
fn require_validatable(state: &MigrationState) -> Result<(), crate::MemoryError> {
    if state.phase != Phase::Prepared && state.phase != Phase::DestinationValidated {
        return Err(query_error(format!(
            "the journal stands at {:?}; validation runs before the switch, \
             not after it",
            state.phase
        )));
    }
    for (name, progress) in &state.progress {
        if *progress != CollectionProgress::Complete {
            return Err(query_error(format!(
                "collection '{name}' stands at {progress:?}; an unfinished \
                 rebuild cannot be validated — resume it first"
            )));
        }
    }
    Ok(())
}

/// One store as the validation reads it: the database handle and the agent
/// view the edge export goes through, opened together because neither is
/// meaningful for this pass without the other.
struct StoreView {
    db: std::sync::Arc<Database>,
    memory: AgentMemory,
}

impl StoreView {
    /// The source opens its `AgentMemory` at its OWN width, discovered from
    /// the store — the fact walk does not care, but the edge export does, and
    /// under `reembed` the source's width is not the target's.
    fn open_source(dir: &Path, target_dimension: usize) -> Result<Self, crate::MemoryError> {
        let db = std::sync::Arc::new(Database::open(dir)?);
        let dimension = db
            .get_any_collection(AGENT_COLLECTIONS[0])
            .map_or(target_dimension, |collection| collection.config().dimension);
        let memory = AgentMemory::with_dimension(std::sync::Arc::clone(&db), dimension)?;
        Ok(Self { db, memory })
    }

    /// The destination was built at the target's width, and says so.
    fn open_destination(dir: &Path, target_dimension: usize) -> Result<Self, crate::MemoryError> {
        let db = std::sync::Arc::new(Database::open(dir)?);
        let memory = AgentMemory::with_dimension(std::sync::Arc::clone(&db), target_dimension)?;
        Ok(Self { db, memory })
    }

    fn facts(
        &self,
        collection: &str,
        batch: usize,
    ) -> Result<BTreeMap<u64, serde_json::Value>, crate::MemoryError> {
        let mut facts = BTreeMap::new();
        for fact in enumerate_by_cursor(&self.db, collection, batch)? {
            let payload: serde_json::Value =
                serde_json::from_str(&fact.payload).map_err(|err| {
                    query_error(format!(
                        "fact {} in '{collection}' carries unreadable payload: {err}",
                        fact.id
                    ))
                })?;
            facts.insert(fact.id, payload);
        }
        Ok(facts)
    }

    fn edges(&self, collection: &str, batch: usize) -> Result<Vec<GraphEdge>, crate::MemoryError> {
        export_edges_verified(&self.memory, &self.db, collection, batch)
    }

    /// See [`divergence_explained_by_expiry`] for why absence means expiry.
    fn vanished(&self, collection: &str, id: u64) -> bool {
        divergence_explained_by_expiry(&self.db, collection, id)
    }
}

/// Which of the two stores an observation belongs to — named, because a
/// refusal that cannot say WHICH side held the stray fact is half a refusal.
#[derive(Debug, Clone, Copy)]
enum Side {
    Source,
    Destination,
}

impl Side {
    fn name(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Destination => "destination",
        }
    }
}

/// One collection compared across the two views, accumulating into the
/// outcome. A struct rather than a parameter list, because every method needs
/// the same five things and a signature repeating them five times is where
/// the sixth one gets threaded through wrongly.
struct Comparison<'a> {
    source: &'a StoreView,
    destination: &'a StoreView,
    collection: &'a str,
    batch: usize,
    outcome: &'a mut ValidationOutcome,
}

impl Comparison<'_> {
    fn run(&mut self) -> Result<(), crate::MemoryError> {
        self.compare_facts()?;
        self.compare_edges()
    }

    fn view(&self, side: Side) -> &StoreView {
        match side {
            Side::Source => self.source,
            Side::Destination => self.destination,
        }
    }

    fn compare_facts(&mut self) -> Result<(), crate::MemoryError> {
        let source_facts = self.source.facts(self.collection, self.batch)?;
        let destination_facts = self.destination.facts(self.collection, self.batch)?;
        self.outcome.facts += source_facts.len() as u64;

        for (id, payload) in &source_facts {
            self.compare_one_fact(*id, payload, destination_facts.get(id))?;
        }
        for id in destination_facts.keys() {
            if !source_facts.contains_key(id) {
                self.fact_explained_or_loss(Side::Destination, *id)?;
            }
        }
        Ok(())
    }

    /// One source fact against what the destination holds under the same id.
    fn compare_one_fact(
        &mut self,
        id: u64,
        payload: &serde_json::Value,
        found: Option<&serde_json::Value>,
    ) -> Result<(), crate::MemoryError> {
        match found {
            Some(found) if found == payload => Ok(()),
            Some(_) => Err(query_error(format!(
                "fact {id} in '{}' differs between source and destination; a \
                 payload that changed in transit is loss, and no expiry \
                 explains a fact both stores still hold",
                self.collection
            ))),
            None => self.fact_explained_or_loss(Side::Source, id),
        }
    }

    /// A diverging id either explains itself by expiry or fails the pass.
    fn fact_explained_or_loss(&mut self, side: Side, id: u64) -> Result<(), crate::MemoryError> {
        if self.view(side).vanished(self.collection, id) {
            self.outcome.explained_by_expiry += 1;
            return Ok(());
        }
        Err(query_error(format!(
            "fact {id} in '{}' exists only on the {} side and is still live \
             there; this is loss, not a clock window",
            self.collection,
            side.name(),
        )))
    }

    fn compare_edges(&mut self) -> Result<(), crate::MemoryError> {
        let exported = self.source.edges(self.collection, self.batch)?;
        let back = self.destination.edges(self.collection, self.batch)?;
        self.outcome.edges += exported.len() as u64;

        let source_tuples = edge_map(&exported);
        let destination_tuples = edge_map(&back);
        self.sweep_missing_or_changed(&source_tuples, &destination_tuples, &exported)?;
        self.sweep_surplus(&source_tuples, &destination_tuples, &back)
    }

    /// Source edges the destination lacks or holds differently.
    fn sweep_missing_or_changed(
        &mut self,
        source_tuples: &BTreeMap<u64, EdgeTuple>,
        destination_tuples: &BTreeMap<u64, EdgeTuple>,
        exported: &[GraphEdge],
    ) -> Result<(), crate::MemoryError> {
        for (id, tuple) in source_tuples {
            if destination_tuples.get(id) != Some(tuple) {
                self.edge_explained_or_loss(Side::Source, exported, *id)?;
            }
        }
        Ok(())
    }

    /// Destination edges the source never exported.
    fn sweep_surplus(
        &mut self,
        source_tuples: &BTreeMap<u64, EdgeTuple>,
        destination_tuples: &BTreeMap<u64, EdgeTuple>,
        back: &[GraphEdge],
    ) -> Result<(), crate::MemoryError> {
        for id in destination_tuples.keys() {
            if !source_tuples.contains_key(id) {
                self.edge_explained_or_loss(Side::Destination, back, *id)?;
            }
        }
        Ok(())
    }

    /// A diverging edge explains itself iff one of its endpoints expired.
    fn edge_explained_or_loss(
        &mut self,
        side: Side,
        edges: &[GraphEdge],
        id: u64,
    ) -> Result<(), crate::MemoryError> {
        let Some(edge) = edges.iter().find(|edge| edge.id() == id) else {
            return Err(query_error(format!(
                "edge {id} in '{}' diverges and its tuple is not in the export \
                 that reported it; the comparison itself is inconsistent",
                self.collection
            )));
        };
        let holder = self.view(side);
        if holder.vanished(self.collection, edge.source())
            || holder.vanished(self.collection, edge.target())
        {
            self.outcome.explained_by_expiry += 1;
            return Ok(());
        }
        Err(query_error(format!(
            "edge {id} ({} -{}-> {}) in '{}' diverges between source and \
             destination and both endpoints are still live; this is loss, not \
             a clock window",
            edge.source(),
            edge.label(),
            edge.target(),
            self.collection,
        )))
    }
}

type EdgeTuple = (u64, u64, String, String);

/// Edges keyed by id, each reduced to an orderable tuple with its properties
/// rendered through a `BTreeMap` for a stable order.
fn edge_map(edges: &[GraphEdge]) -> BTreeMap<u64, EdgeTuple> {
    edges
        .iter()
        .map(|edge| {
            let properties: BTreeMap<_, _> = edge.properties().iter().collect();
            (
                edge.id(),
                (
                    edge.source(),
                    edge.target(),
                    edge.label().to_owned(),
                    serde_json::to_string(&properties).unwrap_or_default(),
                ),
            )
        })
        .collect()
}

/// Whether an id one of THIS validation's walks returned now reads back as
/// absent — which, under the lock this validation holds, can only mean its
/// absolute expiry passed between the walk and this probe.
///
/// The reasoning is deliberately indirect, because it has to be: an expired
/// point is invisible on EVERY public read surface — `get` answers `None` for
/// it exactly as for a deleted one — so its expiry cannot be read back
/// directly. What makes `None` conclusive here is the flock: this validation
/// holds both stores open for its whole pass, nothing else can write or
/// delete under it, and so the only mover left between a walk that saw the id
/// and a probe that does not is the clock crossing the point's own
/// `_veles_expires_at`.
///
/// The contract is therefore narrow: call this ONLY with ids a walk of this
/// same session returned. An arbitrary id the store never held also reads
/// back `None`, and this function cannot tell the two apart — the caller's
/// provenance of the id is what gives the answer its meaning.
pub(crate) fn divergence_explained_by_expiry(db: &Database, collection: &str, id: u64) -> bool {
    let Some(any) = db.get_any_collection(collection) else {
        return false;
    };
    !matches!(any.get(&[id]).into_iter().next(), Some(Some(_)))
}

fn query_error(message: impl Into<String>) -> crate::MemoryError {
    velesdb_core::Error::Query(message.into()).into()
}
