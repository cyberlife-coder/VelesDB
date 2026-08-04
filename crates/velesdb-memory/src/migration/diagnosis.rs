use super::enumeration::{enumerate_by_cursor, scroll_page, AGENT_COLLECTIONS};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Whether a capability the rebuild depends on is established, or missing.
///
/// `Missing` is a full stop, not a warning: PR B does not start while one is
/// outstanding, and no identifier mapping is invented to work around it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Capability {
    /// Established by running it, with the evidence that established it.
    Proven {
        /// What was run, and what it produced.
        evidence: String,
    },
    /// Not available, with the blocker named.
    Missing {
        /// Why the rebuild cannot rely on this.
        blocker: String,
    },
}

impl Capability {
    /// Whether this capability may be relied on.
    #[must_use]
    pub fn is_proven(&self) -> bool {
        matches!(self, Self::Proven { .. })
    }
}

// ---------------------------------------------------------------------------
// THE DIAGNOSIS
// ---------------------------------------------------------------------------

/// The shape of a [`DiagnosisReport`], stamped into every report.
///
/// A report is read back by a later run — possibly a later *binary* — to decide
/// whether a prepared migration may resume. A report whose version this build
/// does not understand is refused rather than guessed at, which is only
/// possible because the number travels with the data.
pub const DIAGNOSIS_FORMAT_VERSION: u32 = 1;

/// What the store itself records about the embedder that filled it.
///
/// `Unknown` is the NOMINAL case, not a fault: every store created before
/// `embedding-provenance.json` existed has no record, and the one this daemon
/// actually runs on is one of them. Reporting `Unknown` honestly is the whole
/// point — a diagnosis that invented a model would be trusted.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceProvenance {
    /// The store records the model it was filled by.
    Known {
        /// The recorded model identifier.
        model: String,
        /// The width that model produces.
        dimension: usize,
    },
    /// The store records nothing, and why that is expected.
    Unknown {
        /// What was looked for, and what its absence does and does not mean.
        reason: String,
    },
}

/// What the expiries in a collection amount to.
///
/// Counted from the payloads rather than from the in-memory TTL map, because
/// the map is rebuilt from those payloads on open and a diagnosis must describe
/// the DISK, not a derived view of it.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct TtlSummary {
    /// Facts carrying an absolute `_veles_expires_at`.
    pub with_expiry: u64,
    /// The soonest expiry, as the absolute unix second stored.
    pub earliest: Option<u64>,
    /// The furthest expiry, as the absolute unix second stored.
    pub latest: Option<u64>,
}

impl TtlSummary {
    /// Fold one observed expiry in.
    fn observe(&mut self, expires_at: u64) {
        self.with_expiry += 1;
        self.earliest = Some(self.earliest.map_or(expires_at, |e| e.min(expires_at)));
        self.latest = Some(self.latest.map_or(expires_at, |e| e.max(expires_at)));
    }

    /// Fold a whole collection's summary into a store-wide one.
    fn merge(&mut self, other: &Self) {
        self.with_expiry += other.with_expiry;
        if let Some(e) = other.earliest {
            self.earliest = Some(self.earliest.map_or(e, |cur| cur.min(e)));
        }
        if let Some(l) = other.latest {
            self.latest = Some(self.latest.map_or(l, |cur| cur.max(l)));
        }
    }
}

/// One collection as the rebuild will find it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CollectionInventory {
    /// The collection name.
    pub name: String,
    /// Whether it exists at all. A store missing one of the three is a store
    /// `AgentMemory` would CREATE the missing one in — a write, and so a thing
    /// a diagnosis must report rather than trigger.
    pub present: bool,
    /// The width its vectors are stored at, `None` when absent.
    pub dimension: Option<usize>,
    /// Live facts, counted by walking the cursor — not read off `point_count`,
    /// which counts what the config believes.
    pub facts: u64,
    /// Live edges, or `None` when the offline route could not establish them.
    pub edges: Option<u64>,
    /// Facts marked as saved working contexts.
    pub working_contexts: u64,
    /// Every reserved key actually observed in a payload. The rebuild has to
    /// carry each one through verbatim, so an unenumerated one is a silent
    /// loss waiting to happen.
    pub reserved_metadata: BTreeSet<String>,
    /// What the expiries here amount to.
    pub ttl: TtlSummary,
}

impl CollectionInventory {
    /// A collection the store does not have.
    fn absent(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            present: false,
            dimension: None,
            facts: 0,
            edges: None,
            working_contexts: 0,
            reserved_metadata: BTreeSet::new(),
            ttl: TtlSummary::default(),
        }
    }

    /// Whether this collection holds no facts.
    ///
    /// Distinct from [`Self::present`] on purpose: an empty collection that
    /// EXISTS still pins the store's dimension and still blocks the open, so a
    /// report that conflated the two would under-state the work.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.facts == 0
    }
}

/// Everything a rebuild needs to know about a store, and nothing it could act
/// on by accident.
///
/// This is deliberately NOT a migration state. A state says "a migration is
/// under way, here is how far it got" and is written to disk; producing one
/// from a diagnosis would turn a question into a commitment. A report answers
/// "what is here, and what could go wrong" and is returned, never written.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DiagnosisReport {
    /// The shape of this report — see [`DIAGNOSIS_FORMAT_VERSION`].
    pub format_version: u32,
    /// The store that was inspected.
    pub source_path: PathBuf,
    /// A digest of the store's files, so a resume can tell whether the source
    /// changed under it. See [`fingerprint`].
    pub source_fingerprint: String,
    /// The width the store's collections are at, `None` when they disagree or
    /// the store has none.
    pub source_dimension: Option<usize>,
    /// What the store records about its embedder.
    pub source_provenance: SourceProvenance,
    /// The model the rebuild would target.
    pub target_model: String,
    /// The width that model produces.
    pub target_dimension: usize,
    /// One entry per collection in [`AGENT_COLLECTIONS`], absent ones included.
    pub collections: Vec<CollectionInventory>,
    /// Live facts across the store.
    pub facts: u64,
    /// Live edges across the store, `0` when none were established — read
    /// alongside the `edges` capability, which says whether that `0` is a
    /// count or an absence.
    pub edges: u64,
    /// Saved working contexts across the store.
    pub working_contexts: u64,
    /// Every reserved key observed anywhere in the store.
    pub reserved_metadata: BTreeSet<String>,
    /// What the expiries across the store amount to.
    pub ttl_summary: TtlSummary,
    /// What the store occupies, summed over its files.
    pub bytes_on_disk: u64,
    /// Free space at the destination, or `None` when it could not be
    /// established — see the `disk_headroom` blocker.
    pub disk_headroom: Option<u64>,
    /// Whether destination and source sit on one filesystem, `None` when there
    /// is no destination to compare or the platform does not say.
    pub same_filesystem: Option<bool>,
    /// What the rebuild may rely on, and what it may not.
    pub capabilities: BTreeMap<String, Capability>,
    /// Everything that must be settled before PR B starts.
    pub blockers: Vec<String>,
}

impl DiagnosisReport {
    /// Whether a rebuild could proceed with no outstanding question.
    ///
    /// Expect `false`, and read that as information rather than as a fault: the
    /// `source_open_is_read_only` blocker stands on every store, because
    /// opening one rewrites its derived index files. A rebuild in place is
    /// therefore never clear — which is the finding, not a bug in the check.
    #[must_use]
    pub fn is_clear(&self) -> bool {
        self.blockers.is_empty() && self.capabilities.values().all(Capability::is_proven)
    }
}

/// A digest of every file in `dir`, over paths and lengths.
///
/// Deliberately NOT a cryptographic hash and deliberately not `DefaultHasher`:
/// the first would cost a full read of the store to answer a question about
/// accidental change, and the second is explicitly not stable across Rust
/// releases — a fingerprint taken by one binary and compared by the next has to
/// mean the same thing. FNV-1a over the sorted `(relative path, length)` pairs
/// is fixed by this function and by nothing else.
///
/// What it detects: a file added, removed, or resized between prepare and
/// resume. What it does not: an in-place edit that preserves length, or
/// tampering by anyone who can also recompute this. It is a guard against a
/// store that moved on, not against an adversary.
///
/// # Errors
/// Returns [`crate::MemoryError`] if the directory cannot be walked.
pub fn fingerprint(dir: &Path) -> Result<String, crate::MemoryError> {
    let mut entries: Vec<(String, u64)> = Vec::new();
    collect_files(dir, dir, &mut entries)?;
    entries.sort_unstable();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for (path, len) in &entries {
        for byte in path.as_bytes().iter().chain(&len.to_le_bytes()) {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    Ok(format!("fnv1a64:{hash:016x}"))
}

/// Sum of every file's length under `dir`.
///
/// # Errors
/// Returns [`crate::MemoryError`] if the directory cannot be walked.
pub fn bytes_on_disk(dir: &Path) -> Result<u64, crate::MemoryError> {
    let mut entries: Vec<(String, u64)> = Vec::new();
    collect_files(dir, dir, &mut entries)?;
    Ok(entries.iter().map(|(_, len)| len).sum())
}

/// Walk `dir` recursively, recording each file's path relative to `root` and
/// its length.
fn collect_files(
    root: &Path,
    dir: &Path,
    out: &mut Vec<(String, u64)>,
) -> Result<(), crate::MemoryError> {
    let read = std::fs::read_dir(dir)
        .map_err(|e| velesdb_core::Error::Query(format!("cannot read {}: {e}", dir.display())))?;
    for entry in read {
        let entry =
            entry.map_err(|e| velesdb_core::Error::Query(format!("cannot read an entry: {e}")))?;
        let path = entry.path();
        let meta = entry.metadata().map_err(|e| {
            velesdb_core::Error::Query(format!("cannot stat {}: {e}", path.display()))
        })?;
        if meta.is_dir() {
            collect_files(root, &path, out)?;
        } else {
            let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy();
            out.push((rel.into_owned(), meta.len()));
        }
    }
    Ok(())
}

/// Whether `a` and `b` sit on the same filesystem.
///
/// The destination normally does NOT exist yet — that is the whole point of
/// asking before creating it — so each path is resolved to its deepest existing
/// ancestor first. Stat-ing the destination itself would answer "unknown" for
/// every question actually worth asking.
///
/// `None` off unix, where the standard library exposes no device id: a rename
/// across filesystems fails where one within a filesystem does not, so a
/// migration that assumed "same" would discover it at switch-over time. Saying
/// "unknown" keeps that decision with the operator.
#[must_use]
pub fn same_filesystem(a: &Path, b: &Path) -> Option<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let (a, b) = (existing_ancestor(a)?, existing_ancestor(b)?);
        Some(a.dev() == b.dev())
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        None
    }
}

/// Metadata of `path`, or of the nearest ancestor that exists.
#[cfg(unix)]
fn existing_ancestor(path: &Path) -> Option<std::fs::Metadata> {
    path.ancestors().find_map(|p| std::fs::metadata(p).ok())
}

/// The batch size the inventory walks with — large enough that the walk is not
/// dominated by per-batch overhead, small enough that a store far bigger than
/// memory is still read in bounded chunks.
const INVENTORY_BATCH: usize = 1024;

/// Inspect `source` and report what a rebuild onto `target_model` would face.
///
/// Reads. Only reads. It opens the store — which takes the exclusive lock, so
/// the daemon must be down — walks each collection by cursor, and returns. It
/// creates no destination, writes no state, and renames nothing; `destination`
/// is used to ask which filesystem it would sit on, and is not created.
///
/// # Errors
/// Returns [`crate::MemoryError`] if the store cannot be read or walked.
pub fn diagnose(
    source: &Path,
    target_model: &str,
    target_dimension: usize,
    destination: Option<&Path>,
) -> Result<DiagnosisReport, crate::MemoryError> {
    let source_fingerprint = fingerprint(source)?;
    let bytes = bytes_on_disk(source)?;
    let source_provenance = read_provenance(source);

    let db = std::sync::Arc::new(velesdb_core::Database::open(source)?);
    let mut collections: Vec<CollectionInventory> = AGENT_COLLECTIONS
        .iter()
        .map(|name| inventory_collection(&db, name))
        .collect::<Result<_, _>>()?;
    let source_dimension = agreed_dimension(&collections);
    let edges_capability = attach_edge_counts(&db, &mut collections, source_dimension);

    let mut totals = Totals::default();
    for inv in &collections {
        totals.fold(inv);
    }
    let (capabilities, blockers) =
        capabilities_and_blockers(&collections, &source_provenance, edges_capability);

    Ok(DiagnosisReport {
        format_version: DIAGNOSIS_FORMAT_VERSION,
        source_path: source.to_path_buf(),
        source_fingerprint,
        source_dimension,
        source_provenance,
        target_model: target_model.to_owned(),
        target_dimension,
        collections,
        facts: totals.facts,
        edges: totals.edges,
        working_contexts: totals.working_contexts,
        reserved_metadata: totals.reserved_metadata,
        ttl_summary: totals.ttl,
        bytes_on_disk: bytes,
        disk_headroom: None,
        same_filesystem: destination.and_then(|dest| same_filesystem(source, dest)),
        capabilities,
        blockers,
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

impl Totals {
    fn fold(&mut self, inv: &CollectionInventory) {
        self.facts += inv.facts;
        self.edges += inv.edges.unwrap_or(0);
        self.working_contexts += inv.working_contexts;
        self.reserved_metadata
            .extend(inv.reserved_metadata.iter().cloned());
        self.ttl.merge(&inv.ttl);
    }
}

/// What the store records about its embedder, phrased so an absent record
/// cannot be misread as a match.
fn read_provenance(source: &Path) -> SourceProvenance {
    match crate::embedding_provenance::read(source) {
        Ok(Some(p)) => SourceProvenance::Known {
            model: p.model,
            dimension: p.dimension,
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
    let mut inv = CollectionInventory {
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
            fold_payload(&mut inv, &fact.payload);
        }
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    Ok(inv)
}

/// Fold one stored payload into the collection's tallies.
fn fold_payload(inv: &mut CollectionInventory, payload: &str) {
    inv.facts += 1;
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(payload) else {
        return;
    };
    for key in map.keys() {
        if crate::storage::is_reserved_key(key) {
            inv.reserved_metadata.insert(key.clone());
        }
    }
    if map.contains_key(crate::storage::CTX_WORKING_FIELD) {
        inv.working_contexts += 1;
    }
    if let Some(expiry) = map
        .get(velesdb_core::collection::EXPIRES_AT_KEY)
        .and_then(Value::as_u64)
    {
        inv.ttl.observe(expiry);
    }
}

/// The one width every present collection shares, or `None` when they disagree.
///
/// They disagreeing is not a theoretical case to shrug at: `AgentMemory` opens
/// all three at ONE dimension, so a store whose collections drifted apart
/// cannot be opened by it at all, and the rebuild has no single source width to
/// read from.
fn agreed_dimension(collections: &[CollectionInventory]) -> Option<usize> {
    let mut dims = collections.iter().filter_map(|c| c.dimension);
    let first = dims.next()?;
    dims.all(|d| d == first).then_some(first)
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
    db: &std::sync::Arc<velesdb_core::Database>,
    collections: &mut [CollectionInventory],
    source_dimension: Option<usize>,
) -> Capability {
    if !collections.iter().all(|c| c.present) {
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
    db: &std::sync::Arc<velesdb_core::Database>,
    collections: &mut [CollectionInventory],
    dimension: usize,
) -> Result<u64, crate::MemoryError> {
    let memory =
        velesdb_core::agent::AgentMemory::with_dimension(std::sync::Arc::clone(db), dimension)?;
    let mut total = 0u64;
    for inv in collections.iter_mut() {
        let ids: Vec<u64> = enumerate_by_cursor(db, &inv.name, INVENTORY_BATCH)?
            .into_iter()
            .map(|f| f.id)
            .collect();
        let mut count = 0u64;
        for id in ids {
            count += edges_of(&memory, &inv.name, id)?;
        }
        inv.edges = Some(count);
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

/// `Database::open` is not inert, and the rebuild has to be told so.
const WRITE_ON_OPEN: &str =
    "`Database::open` REWRITES derived artifacts — the HNSW index, the id mappings, the \
     collection meta, the vector index and the vector WAL — on the first open that follows a \
     write session, before a single fact is read. Measured by isolation: the open alone changes \
     those files, while the cursor walk and `AgentMemory` construction that follow change \
     nothing, and a second open of the now-normalised store changes nothing either. So a \
     diagnosis is read-only with respect to the DATA and is not byte-for-byte inert on the \
     DIRECTORY: it must be run against a controlled copy, never against the store being relied \
     on. This is a permanent property of the engine, not a per-store finding.";

/// Free space cannot be read, and a rebuild needs it.
const NO_HEADROOM: &str =
    "free space is not established: the standard library exposes no free-space API and this \
     crate depends on nothing that does. A rebuild into a separate destination needs at least \
     `bytes_on_disk` free, so without this the shortfall would surface mid-rebuild instead of \
     before it.";

/// The embedder's cost per fact is the one number a rebuild's duration turns on
/// and the one this gate cannot produce.
const NO_EMBEDDER_COST: &str =
    "the embedding cost per fact is NOT established. Everything measured so far is the STORE's      cost — 16.3 us/fact to re-insert, once #1797 removed the per-document fsync — and that is      the smaller half. Re-embedding calls a model over a network, at a cost set by the model,      the backend and the hardware, none of which a unit test may depend on without becoming a      test of whether Ollama happens to be running. Stated rather than guessed: a rebuild's      duration is dominated by this unknown, and it has to be measured against the actual target      model before any duration is promised to an operator.";

/// An unrecorded source model makes an equal-width swap invisible.
const NO_PROVENANCE: &str =
    "the source model is not recorded, so a model change at EQUAL width cannot be detected — the \
     vectors would be silently incomparable. The operator has to state the source model; it \
     cannot be discovered.";

/// What the rebuild may rely on, and what must be settled first.
fn capabilities_and_blockers(
    collections: &[CollectionInventory],
    provenance: &SourceProvenance,
    edges: Capability,
) -> (BTreeMap<String, Capability>, Vec<String>) {
    let capabilities = capability_map(provenance, edges);
    let blockers = capabilities
        .iter()
        .filter_map(|(name, cap)| match cap {
            Capability::Missing { blocker } => Some(format!("{name}: {blocker}")),
            Capability::Proven { .. } => None,
        })
        .chain(
            collections
                .iter()
                .filter(|c| !c.present)
                .map(|c| format!("collection `{}` is absent from the store", c.name)),
        )
        .collect();
    (capabilities, blockers)
}

/// Every capability the rebuild depends on, with its verdict.
fn capability_map(
    provenance: &SourceProvenance,
    edges: Capability,
) -> BTreeMap<String, Capability> {
    let missing = |blocker: &str| Capability::Missing {
        blocker: blocker.to_owned(),
    };
    let mut capabilities = BTreeMap::new();
    capabilities.insert("edges".to_owned(), edges);
    capabilities.insert(
        "inventory".to_owned(),
        Capability::Proven {
            evidence: format!(
                "all {} agent collections were looked up by name and walked by cursor; absent \
                 ones are reported as absent rather than skipped.",
                AGENT_COLLECTIONS.len()
            ),
        },
    );
    capabilities.insert(
        "source_open_is_read_only".to_owned(),
        missing(WRITE_ON_OPEN),
    );
    capabilities.insert("disk_headroom".to_owned(), missing(NO_HEADROOM));
    capabilities.insert("embedder_cost".to_owned(), missing(NO_EMBEDDER_COST));
    if matches!(provenance, SourceProvenance::Unknown { .. }) {
        capabilities.insert("source_provenance".to_owned(), missing(NO_PROVENANCE));
    }
    capabilities
}
