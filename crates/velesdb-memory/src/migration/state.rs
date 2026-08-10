use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};

mod lock;
mod resume;
mod switch;

#[cfg(test)]
pub(super) use lock::LOCK_GUARD_FILE;
pub use lock::{MigrationLock, LOCK_FILE};
pub use switch::{Phase, Recovery, SwitchState, PHASES};

// ---------------------------------------------------------------------------
// THE LOCK AND THE PHASE JOURNAL
//
// A rebuild is a sequence that can stop anywhere: between reading and writing,
// between writing and validating, between archiving the source and activating
// the destination. What matters is not that it never stops — it is that every
// place it CAN stop has one defined action, and that an ambiguous stop changes
// nothing at all.
// ---------------------------------------------------------------------------

/// The file a prepared migration records its state in.
pub const STATE_FILE: &str = "migration-state.json";

/// The fixed sibling staging file for an atomic state replacement.
///
/// Its presence is ambiguous evidence of an interrupted writer, so it is
/// never overwritten or silently swept by a later run.
pub const STATE_TEMP_FILE: &str = "migration-state.json.tmp";

/// The shape of a [`MigrationState`].
///
/// Bumped when the state's meaning changes. Only the current version may
/// resume: a newer state may contain unknown decisions, while an older one may
/// rely on guarantees this build deliberately strengthened.
///
/// # v3 — per-collection rebuild progress (#1762, PR C2b)
///
/// v2 recorded the phase and nothing else, so a resumed rebuild had to start
/// every collection from zero. v3 adds [`MigrationState::progress`], and with
/// it two rules a v2 build never enforced: progress can only advance, and the
/// phase cannot leave [`Phase::Prepared`] while any collection is unfinished.
pub const STATE_FORMAT_VERSION: u32 = 3;

/// How far one collection's rebuild got inside [`Phase::Prepared`].
///
/// Three stages, because a resume needs to answer three different questions.
/// `Facts` says where the cursor walk stands — `cursor` is the last fact id
/// reinserted, and `None` means the walk has not started. `Edges` says every
/// fact landed and the edge pass is running; it carries no cursor because the
/// pass is idempotent end to end (reinserting an existing edge answers with
/// the same id, and the destination is verified by re-reading, so replaying it
/// after a crash is safe where replaying half a fact walk would not be).
/// `Complete` says both are done and a resume must not touch the collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum CollectionProgress {
    /// The fact walk is under way, resumable strictly after `cursor`.
    Facts {
        /// The last fact id reinserted at the destination; `None` = not started.
        cursor: Option<u64>,
    },
    /// Every fact landed; the (idempotent) edge pass runs until `Complete`.
    Edges,
    /// Facts and edges are both at the destination.
    Complete,
}

impl CollectionProgress {
    /// Whether `self` may be recorded after `previous` for one collection.
    ///
    /// Exactly the transitions the pass emits, and no others. An earlier
    /// version tolerated skipping `Edges` ("a collection with no edges goes
    /// straight to Complete") — which was FALSE: the pass journals `Edges`
    /// unconditionally, edges or none. A tolerance the writer never uses only
    /// widens what a BUGGY writer can make the journal swallow — a refactor
    /// that lost the edge pass would have journalled `Complete` without a
    /// sound. Requiring the passage through `Edges` costs the real writer
    /// nothing and makes that bug un-journallable.
    fn may_follow(self, previous: Self) -> bool {
        match (previous, self) {
            (Self::Facts { cursor: before }, Self::Facts { cursor: after }) => {
                match (before, after) {
                    (Some(before), Some(after)) => after >= before,
                    (Some(_), None) => false,
                    (None, _) => true,
                }
            }
            (Self::Facts { .. } | Self::Edges, Self::Edges)
            | (Self::Edges | Self::Complete, Self::Complete) => true,
            _ => false,
        }
    }
}

/// What a prepared migration recorded, so a later run can decide whether to
/// resume it.
///
/// Emphatically not a [`crate::migration::DiagnosisReport`]: a report answers
/// "what is here", a
/// state asserts "a migration is under way and got this far". A diagnosis never
/// produces one.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MigrationState {
    /// The shape of this state — see [`STATE_FORMAT_VERSION`].
    pub format_version: u32,
    /// How far the migration got.
    pub phase: Phase,
    /// The store being migrated.
    pub source_path: PathBuf,
    /// The source's fingerprint when the migration was prepared.
    pub source_fingerprint: String,
    /// The model the migration is rebuilding against.
    pub target_model: String,
    /// The width that model produces.
    pub target_dimension: usize,
    /// How far each collection's rebuild got — exactly one entry per agent
    /// collection, always. A missing key would silently skip a collection's
    /// rebuild; an extra one would journal work nobody will do. Both are
    /// refused by validation rather than tolerated.
    pub progress: std::collections::BTreeMap<String, CollectionProgress>,
    /// A digest of what the target embedder actually PRODUCES, not what it is
    /// called — `sha256:` over the vector it answers for a fixed sentinel
    /// sentence. `Some` exactly when the resolved regime is `reembed`, `None`
    /// under `reuse`, so the field also records the regime without a second
    /// field to drift from it.
    ///
    /// This exists because [`MigrationState::may_resume`]'s model check
    /// compares NAMES, and a name is a claim: `ollama pull` updates a model's
    /// weights in place under the same identifier. A run resumed across such
    /// an update would collide its replayed batch into run-one vectors and
    /// write run-two vectors after it — one store, one recorded model, two
    /// incompatible vector spaces, which is exactly what the model check says
    /// it prevents. Vectors do not lie about the embedder that made them.
    pub embedder_witness: Option<String>,
}

impl MigrationState {
    /// Whether this state may be resumed against the source and target now in
    /// front of us.
    ///
    /// Every refusal names both sides. A resume that silently adapted to a
    /// changed fingerprint would rebuild from a store that is no longer the one
    /// it inventoried; one that adapted to a changed model would produce a
    /// store whose vectors and whose recorded model disagree.
    ///
    /// # Errors
    /// A message naming what changed and what the operator can do about it.
    pub fn may_resume(
        &self,
        source_path: &Path,
        source_fingerprint: &str,
        target_model: &str,
        target_dimension: usize,
    ) -> Result<(), String> {
        validate_current_state_version(self)?;
        validate_state_semantics(self)?;
        validate_migration_identity(
            source_path,
            source_fingerprint,
            target_model,
            target_dimension,
        )
        .map_err(|reason| {
            format!("cannot resume against an invalid requested identity: {reason}")
        })?;
        resume::validate_resume_source(self, source_path)?;
        resume::validate_resume_fingerprint(self, source_fingerprint)?;
        resume::validate_resume_model(self, target_model)?;
        resume::validate_resume_dimension(self, target_dimension)?;
        Ok(())
    }

    /// Read a state from `workspace`, refusing one this build cannot act on.
    ///
    /// The version is read out of the raw JSON BEFORE the state is
    /// deserialised, because a newer state may carry fields this build cannot
    /// parse — and "cannot parse" would otherwise surface as a corruption error
    /// instead of the version refusal it actually is.
    ///
    /// # Errors
    /// The file is unreadable, is not JSON, or is stamped with a version newer
    /// from [`STATE_FORMAT_VERSION`].
    pub fn read(workspace: &Path) -> Result<Option<Self>, String> {
        let Some(value) = read_state_value(workspace)? else {
            return Ok(None);
        };
        let version = serialized_state_version(&value)?;
        validate_serialized_state_version(version)?;
        let state: Self = serde_json::from_value(value).map_err(|err| {
            format!("{STATE_FILE} is version {version} but does not parse: {err}")
        })?;
        validate_state_semantics(&state)
            .map_err(|reason| format!("{STATE_FILE} has invalid semantics: {reason}"))?;
        Ok(Some(state))
    }

    /// Atomically and durably replace the state in `workspace`.
    ///
    /// The caller must hold `lock`. The complete JSON is written to a fixed
    /// sibling staging file, flushed and synced before one atomic promotion.
    /// The promotion is then made durable with the platform's directory or
    /// write-through barrier. A pre-existing staging file is refused as
    /// evidence of an interrupted writer; it is never overwritten.
    ///
    /// # Errors
    /// The lock does not guard this workspace, an existing state is invalid,
    /// staging is ambiguous, or any write/durability step fails.
    pub fn write(&self, workspace: &Path, lock: &MigrationLock) -> Result<(), String> {
        lock.verify_workspace(workspace)?;
        validate_current_state_version(self)?;
        validate_state_semantics(self)?;
        let existing = validate_existing_state(workspace)?;
        validate_state_update(existing.as_ref(), self)?;
        let body = serde_json::to_string_pretty(self)
            .map_err(|err| format!("cannot serialise the migration state: {err}"))?;
        // Re-check ownership immediately before the first mutation. Validation
        // above can be arbitrarily slow on a hostile filesystem; a stale
        // handle must not create even the staging file after an ABA replacement.
        lock.verify_workspace(workspace)?;
        commit_state_with(
            workspace,
            body.as_bytes(),
            promote_state,
            state_durability_barrier,
        )
    }
}

fn read_state_value(workspace: &Path) -> Result<Option<Value>, String> {
    let path = workspace.join(STATE_FILE);
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("cannot read {STATE_FILE}: {err}")),
    };
    serde_json::from_str(&raw)
        .map(Some)
        .map_err(|err| format!("{STATE_FILE} is not readable JSON: {err}"))
}

fn serialized_state_version(value: &Value) -> Result<u64, String> {
    value
        .get("format_version")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{STATE_FILE} carries no format_version"))
}

fn validate_serialized_state_version(version: u64) -> Result<(), String> {
    if version == u64::from(STATE_FORMAT_VERSION) {
        return Ok(());
    }
    let action = if version < u64::from(STATE_FORMAT_VERSION) {
        "This older state predates per-collection rebuild progress; start a fresh diagnosis."
    } else {
        "Use the version that wrote it."
    };
    Err(format!(
        "{STATE_FILE} is version {version} and this build requires version {STATE_FORMAT_VERSION}. Refusing incompatible migration semantics. {action}"
    ))
}

fn validate_current_state_version(state: &MigrationState) -> Result<(), String> {
    if state.format_version == STATE_FORMAT_VERSION {
        return Ok(());
    }
    let action = if state.format_version < STATE_FORMAT_VERSION {
        "This older state predates per-collection rebuild progress. Start a fresh diagnosis."
    } else {
        "Use the version that wrote it."
    };
    Err(format!(
        "this migration state is version {} and this build requires version {}. \
         Resuming across incompatible state semantics is unsafe. {action}",
        state.format_version, STATE_FORMAT_VERSION,
    ))
}

fn validate_state_semantics(state: &MigrationState) -> Result<(), String> {
    validate_migration_identity(
        &state.source_path,
        &state.source_fingerprint,
        &state.target_model,
        state.target_dimension,
    )?;
    validate_progress_keys(state)?;
    validate_phase_against_progress(state)?;
    validate_embedder_witness(state)
}

/// A present witness must be a well-formed digest, not free text an editor
/// could plausibly have typed.
fn validate_embedder_witness(state: &MigrationState) -> Result<(), String> {
    let Some(witness) = &state.embedder_witness else {
        return Ok(());
    };
    let digest = witness
        .strip_prefix("sha256:")
        .filter(|digest| digest.len() == 64)
        .filter(|digest| {
            digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        });
    if digest.is_none() {
        return Err(
            "embedder_witness must be exactly 'sha256:' followed by 64 lowercase hexadecimal \
             characters"
                .to_owned(),
        );
    }
    Ok(())
}

/// The progress map must cover exactly the agent collections.
fn validate_progress_keys(state: &MigrationState) -> Result<(), String> {
    for name in super::enumeration::AGENT_COLLECTIONS {
        if !state.progress.contains_key(*name) {
            return Err(format!(
                "progress carries no entry for collection '{name}'; a resume would \
                 silently skip its rebuild"
            ));
        }
    }
    for name in state.progress.keys() {
        if !super::enumeration::AGENT_COLLECTIONS.contains(&name.as_str()) {
            return Err(format!(
                "progress tracks '{name}', which is not an agent collection; this \
                 journal describes work nobody will do"
            ));
        }
    }
    Ok(())
}

/// A phase past [`Phase::Prepared`] asserts the rebuild is finished, so every
/// collection must say so too — otherwise the journal contradicts itself and
/// a later phase would validate, archive, or activate a half-built store.
fn validate_phase_against_progress(state: &MigrationState) -> Result<(), String> {
    if state.phase == Phase::Prepared {
        return Ok(());
    }
    for (name, progress) in &state.progress {
        if *progress != CollectionProgress::Complete {
            return Err(format!(
                "phase {:?} asserts the rebuild is finished, but collection \
                 '{name}' stands at {progress:?}; the phase cannot leave \
                 {:?} while any collection is unfinished",
                state.phase,
                Phase::Prepared,
            ));
        }
    }
    Ok(())
}

fn validate_migration_identity(
    source_path: &Path,
    source_fingerprint: &str,
    target_model: &str,
    target_dimension: usize,
) -> Result<(), String> {
    if !source_path.is_absolute()
        || source_path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(
            "source_path must be an absolute normalized path produced by diagnosis".to_owned(),
        );
    }
    let digest = source_fingerprint
        .strip_prefix("sha256-tree-v2:")
        .filter(|digest| digest.len() == 64)
        .filter(|digest| {
            digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        });
    if digest.is_none() {
        return Err(
            "source_fingerprint must be exactly 'sha256-tree-v2:' followed by 64 lowercase hexadecimal characters"
                .to_owned(),
        );
    }
    if target_model.trim().is_empty() {
        return Err("target_model must not be empty".to_owned());
    }
    if target_dimension == 0 {
        return Err("target_dimension must be greater than zero".to_owned());
    }
    Ok(())
}

fn validate_state_update(
    existing: Option<&MigrationState>,
    candidate: &MigrationState,
) -> Result<(), String> {
    let Some(existing) = existing else {
        if candidate.phase != Phase::Prepared {
            return Err(format!(
                "a new migration journal must start at {:?}, not {:?}; refusing to invent skipped work",
                Phase::Prepared,
                candidate.phase
            ));
        }
        return Ok(());
    };

    let immutable_drift = if existing.source_path != candidate.source_path {
        Some(format!(
            "source_path changed from '{}' to '{}'",
            existing.source_path.display(),
            candidate.source_path.display()
        ))
    } else if existing.source_fingerprint != candidate.source_fingerprint {
        Some(format!(
            "source_fingerprint changed from '{}' to '{}'",
            existing.source_fingerprint, candidate.source_fingerprint
        ))
    } else if existing.target_model != candidate.target_model {
        Some(format!(
            "target_model changed from '{}' to '{}'",
            existing.target_model, candidate.target_model
        ))
    } else if existing.target_dimension != candidate.target_dimension {
        Some(format!(
            "target_dimension changed from {} to {}",
            existing.target_dimension, candidate.target_dimension
        ))
    } else if existing.embedder_witness != candidate.embedder_witness {
        Some(format!(
            "embedder_witness changed from {:?} to {:?} — either the embedder's \
             output drifted under a stable model name, or the resolved regime \
             flipped between runs; both make the replayed and the remaining \
             batches incompatible",
            existing.embedder_witness, candidate.embedder_witness
        ))
    } else {
        None
    };
    if let Some(drift) = immutable_drift {
        return Err(format!(
            "refusing to rewrite migration identity: {drift}. Start a fresh migration instead"
        ));
    }
    if !candidate.phase.may_follow(existing.phase) {
        return Err(format!(
            "refusing migration phase transition from {:?} to {:?}: journal updates may be idempotent or advance exactly one phase, never regress or skip work",
            existing.phase, candidate.phase
        ));
    }
    validate_progress_advance(existing, candidate)
}

/// Progress may repeat or advance per collection, never regress.
///
/// Key equality between the two maps is already established: both states
/// passed [`validate_progress_keys`] before reaching this comparison.
fn validate_progress_advance(
    existing: &MigrationState,
    candidate: &MigrationState,
) -> Result<(), String> {
    for (name, after) in &candidate.progress {
        let Some(before) = existing.progress.get(name) else {
            continue;
        };
        if !after.may_follow(*before) {
            return Err(format!(
                "refusing progress regression on collection '{name}': the journal \
                 records {before:?} and the update asserts {after:?}; a rebuild \
                 journal may repeat or advance, never regress"
            ));
        }
    }
    Ok(())
}

fn validate_existing_state(workspace: &Path) -> Result<Option<MigrationState>, String> {
    let path = workspace.join(STATE_FILE);
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("cannot inspect existing {STATE_FILE}: {err}")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "refusing to replace {STATE_FILE}: {} is a symlink, directory, or special file",
            path.display()
        ));
    }
    MigrationState::read(workspace)?
        .map(Some)
        .ok_or_else(|| format!("{STATE_FILE} disappeared while it was being validated"))
}

pub(super) fn commit_state_with<P, B>(
    workspace: &Path,
    body: &[u8],
    promote: P,
    durability_barrier: B,
) -> Result<(), String>
where
    P: FnOnce(&Path, &Path) -> std::io::Result<()>,
    B: FnOnce(&Path, &Path) -> std::io::Result<()>,
{
    let temporary = workspace.join(STATE_TEMP_FILE);
    let final_path = workspace.join(STATE_FILE);
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
    {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(format!(
                "refusing to overwrite pre-existing {STATE_TEMP_FILE} at {}: it may be evidence of an interrupted state write; inspect and remove that exact file manually",
                temporary.display()
            ));
        }
        Err(err) => return Err(format!("cannot create {STATE_TEMP_FILE}: {err}")),
    };

    let write_result = (|| {
        file.write_all(body)?;
        file.flush()?;
        file.sync_all()
    })();
    drop(file);
    if let Err(err) = write_result {
        return cleanup_uncommitted_temp(
            &temporary,
            format!("cannot write and sync {STATE_TEMP_FILE}: {err}"),
        );
    }

    if let Err(err) = promote(&temporary, &final_path) {
        return cleanup_uncommitted_temp(
            &temporary,
            format!("cannot atomically promote {STATE_TEMP_FILE} to {STATE_FILE}: {err}"),
        );
    }
    durability_barrier(workspace, &final_path).map_err(|err| {
        format!(
            "{STATE_FILE} was replaced and is visible, but its durability could not be confirmed: {err}. Do not retry blindly; inspect the state before continuing"
        )
    })
}

fn cleanup_uncommitted_temp(temporary: &Path, primary: String) -> Result<(), String> {
    match std::fs::remove_file(temporary) {
        Ok(()) => Err(primary),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Err(primary),
        Err(err) => Err(format!(
            "{primary}; additionally, cannot remove {}: {err}",
            temporary.display()
        )),
    }
}

#[cfg(unix)]
fn promote_state(temporary: &Path, final_path: &Path) -> std::io::Result<()> {
    // Keep promotion and the directory durability barrier as distinct failure
    // domains: after this succeeds, a barrier error must report that the new
    // state is already visible and must not be retried blindly.
    std::fs::rename(temporary, final_path)
}

#[cfg(windows)]
fn promote_state(temporary: &Path, final_path: &Path) -> std::io::Result<()> {
    atomicwrites::replace_atomic(temporary, final_path)
}

#[cfg(not(any(unix, windows)))]
fn promote_state(_temporary: &Path, _final_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "durable migration-state replacement is supported only on Unix and Windows",
    ))
}

#[cfg(unix)]
fn state_durability_barrier(workspace: &Path, _final_path: &Path) -> std::io::Result<()> {
    std::fs::File::open(workspace)?.sync_all()
}

#[cfg(windows)]
fn state_durability_barrier(_workspace: &Path, _final_path: &Path) -> std::io::Result<()> {
    // `atomicwrites::replace_atomic` uses MOVEFILE_WRITE_THROUGH on Windows.
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn state_durability_barrier(_workspace: &Path, _final_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no durable migration-state barrier is defined for this platform",
    ))
}
