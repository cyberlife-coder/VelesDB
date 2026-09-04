use super::diagnostic_copy::DiagnosticCopy;
use super::strategy::{assess, resolve, Resolution, Strategy};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

mod capabilities;
mod inventory;
mod report;

pub(super) fn switch_filesystem_capability(same_filesystem: Option<bool>) -> Capability {
    capabilities::switch_filesystem_capability(same_filesystem)
}

/// Whether a capability the rebuild depends on is established, or missing.
///
/// `Missing` is a full stop, not a warning: PR B does not start while one is
/// outstanding, and no identifier mapping is invented to work around it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
/// # v6 — `edge_export` became `Proven` (#1762, PR C2a)
///
/// The bump is not cosmetic and not optional. A capability's canonical verdict
/// is part of the report's shape: `DiagnosisReport::validate` refuses a report
/// whose `edge_export` disagrees with what this build derives. A v5 report on
/// disk carries the `Missing` verdict that was canonical when it was written,
/// so a v6 build reading it would reject it as *inconsistent* — an accusation
/// about the report's contents, when the truth is that it predates the
/// capability. The version number is what turns that into a clear refusal.
pub const DIAGNOSIS_FORMAT_VERSION: u32 = 6;

/// What the store itself records about the embedder that filled it.
///
/// `Unknown` is the NOMINAL case, not a fault: every store created before
/// `embedding-provenance.json` existed has no record, and the one this daemon
/// actually runs on is one of them. Reporting `Unknown` honestly is the whole
/// point — a diagnosis that invented a model would be trusted.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    /// changed under it. See `fingerprint`.
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
    /// The regime the operator selected — `auto` unless they said otherwise.
    pub requested_strategy: Strategy,
    /// What that request resolves to against this store, and why.
    ///
    /// Derived, never independently observed: `DiagnosisReport::validate`
    /// recomputes it from the provenance and the target contract and refuses a
    /// report whose stated regime does not follow from its own fields. A
    /// diagnosis an operator reads a regime off is a diagnosis that can lie
    /// about one.
    pub resolution: Resolution,
    /// One entry per collection in `AGENT_COLLECTIONS`, absent ones included.
    pub collections: Vec<CollectionInventory>,
    /// Live facts across the store.
    pub facts: u64,
    /// Live edges across the store, `0` when none were established — read
    /// alongside the `edge_counts` capability, which says whether that `0` is a
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
    /// Space required for the verified ephemeral diagnostic copy.
    pub diagnostic_staging_required: u64,
    /// Space observed on the staging volume before any scratch was created.
    pub diagnostic_staging_available: u64,
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

/// Inspect `source` and report what a rebuild onto `target_model` would face.
///
/// The live source is read only with ordinary file handles and may remain held
/// by the daemon. Because [`velesdb_core::Database::open`] rewrites derived
/// files and takes an exclusive lock, it is called only on a verified ephemeral
/// copy under `scratch_parent`. The source is fingerprinted before and after
/// capture and once more after inventory; any movement refuses the report.
/// `destination` is inspected only for filesystem topology and is not created.
///
/// # Errors
/// Returns [`crate::MemoryError`] if the store cannot be read or walked.
pub fn diagnose(
    source: &Path,
    scratch_parent: &Path,
    target: &TargetContract,
    destination: Option<&Path>,
) -> Result<DiagnosisReport, crate::MemoryError> {
    let source = canonical_source(source)?;
    let copy = DiagnosticCopy::capture(&source, scratch_parent)?;
    let result = diagnose_copy(&source, target, destination, &copy);
    copy.finish(result)
}

/// What the operator is pointing the rebuild at: the target embedder's
/// identity, and the regime they selected.
///
/// Grouped rather than passed as three parallel arguments because the three
/// only ever travel together, and because a call site that passed a model and a
/// width belonging to two different embedders would type-check perfectly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetContract {
    /// The model identifier the rebuild would embed with.
    pub model: String,
    /// The width that model produces, as the embedder itself reports it.
    pub dimension: usize,
    /// `auto` unless the operator named a regime.
    pub strategy: Strategy,
}

impl TargetContract {
    /// The usual case: a target embedder, and no opinion about the regime.
    #[must_use]
    pub fn automatic(model: impl Into<String>, dimension: usize) -> Self {
        Self {
            model: model.into(),
            dimension,
            strategy: Strategy::Auto,
        }
    }
}

fn canonical_source(source: &Path) -> Result<PathBuf, crate::MemoryError> {
    let metadata = std::fs::symlink_metadata(source).map_err(|err| {
        velesdb_core::Error::Query(format!(
            "cannot inspect migration source {}: {err}",
            source.display()
        ))
    })?;
    if metadata.file_type().is_symlink() {
        return Err(velesdb_core::Error::Query(format!(
            "migration source {} is a symlink; diagnose the canonical store directory directly",
            source.display()
        ))
        .into());
    }
    let canonical = std::fs::canonicalize(source).map_err(|err| {
        velesdb_core::Error::Query(format!(
            "cannot canonicalize migration source {}: {err}",
            source.display()
        ))
    })?;
    if !canonical.is_absolute() {
        return Err(velesdb_core::Error::Query(format!(
            "canonical migration source is not absolute: {}",
            canonical.display()
        ))
        .into());
    }
    Ok(canonical)
}

pub(super) fn diagnose_copy(
    source: &Path,
    target: &TargetContract,
    destination: Option<&Path>,
    copy: &DiagnosticCopy,
) -> Result<DiagnosisReport, crate::MemoryError> {
    let inventory = inventory::inspect(copy.store_path())?;
    copy.verify_source_unchanged(source)?;
    let same_filesystem = destination.and_then(|dest| same_filesystem(source, dest));
    Ok(report_from_inventory(
        source,
        target,
        same_filesystem,
        copy,
        inventory,
    ))
}

/// The strategy resolution of [`report_from_inventory`]: what the caller
/// asked for, arbitrated against what the store's provenance and dimension
/// actually permit.
fn resolved_strategy(
    target: &TargetContract,
    inventory: &inventory::StoreInventory,
) -> crate::migration::strategy::Resolution {
    resolve(
        target.strategy,
        assess(
            &inventory.source_provenance,
            inventory.source_dimension,
            &target.model,
            target.dimension,
        ),
    )
}

fn report_from_inventory(
    source: &Path,
    target: &TargetContract,
    same_filesystem: Option<bool>,
    copy: &DiagnosticCopy,
    inventory: inventory::StoreInventory,
) -> DiagnosisReport {
    let resolution = resolved_strategy(target, &inventory);
    let capabilities = capabilities::capability_map(
        &inventory.source_provenance,
        inventory.source_dimension,
        &target.model,
        target.dimension,
        inventory.edge_counts,
        same_filesystem,
        copy,
    );
    let blockers = capabilities::blockers_for(&capabilities, &inventory.collections);
    DiagnosisReport {
        format_version: DIAGNOSIS_FORMAT_VERSION,
        source_path: source.to_path_buf(),
        source_fingerprint: copy.source_fingerprint().to_owned(),
        source_dimension: inventory.source_dimension,
        source_provenance: inventory.source_provenance,
        target_model: target.model.clone(),
        target_dimension: target.dimension,
        requested_strategy: target.strategy,
        resolution,
        collections: inventory.collections,
        facts: inventory.facts,
        edges: inventory.edges,
        working_contexts: inventory.working_contexts,
        reserved_metadata: inventory.reserved_metadata,
        ttl_summary: inventory.ttl,
        bytes_on_disk: copy.source_bytes(),
        diagnostic_staging_required: copy.staging_required(),
        diagnostic_staging_available: copy.staging_available(),
        disk_headroom: None,
        same_filesystem,
        capabilities,
        blockers,
    }
}
