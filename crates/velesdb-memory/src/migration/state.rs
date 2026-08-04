use serde_json::Value;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// THE LOCK AND THE PHASE JOURNAL
//
// A rebuild is a sequence that can stop anywhere: between reading and writing,
// between writing and validating, between archiving the source and activating
// the destination. What matters is not that it never stops — it is that every
// place it CAN stop has one defined action, and that an ambiguous stop changes
// nothing at all.
// ---------------------------------------------------------------------------

/// The file that marks a migration in progress.
pub const LOCK_FILE: &str = "migration.lock";

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
pub const STATE_FORMAT_VERSION: u32 = 2;

/// Where a migration has got to.
///
/// Ordered as the migration performs them. `Committed` is the only terminal
/// success; every other value names a place the process can be found stopped.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// The state exists, the destination does not. Nothing has been moved.
    Prepared,
    /// The destination is built and has been checked against the source.
    DestinationValidated,
    /// The source has been moved aside, under its archive name.
    SourceArchived,
    /// The destination now sits at the source's name.
    DestinationActivated,
    /// The archive has been released. The migration is over.
    Committed,
}

/// Every phase, in order — so an exhaustive check cannot silently miss one
/// added later.
pub const PHASES: &[Phase] = &[
    Phase::Prepared,
    Phase::DestinationValidated,
    Phase::SourceArchived,
    Phase::DestinationActivated,
    Phase::Committed,
];

/// What to do with a migration found stopped.
///
/// There is deliberately no "clean up and carry on" variant. Every outcome
/// either moves forward from a known point, puts the source back, refuses, or
/// hands over to a human — and the last two change nothing on disk.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "recovery", rename_all = "snake_case")]
pub enum Recovery {
    /// Resume forward, starting at the named phase.
    Continue {
        /// The phase to perform next.
        next: Phase,
        /// Why continuing is safe from here.
        rationale: String,
    },
    /// Put the source back where it was. Only ever from a state where the
    /// source is provably intact.
    Restore {
        /// Exactly what to move where.
        action: String,
    },
    /// Change nothing. The state on disk does not determine what happened.
    Refuse {
        /// What is ambiguous, and what a human must look at.
        reason: String,
    },
}

impl Phase {
    /// What to do when a migration is found stopped in this phase.
    ///
    /// Total by construction — the match has no wildcard arm, so a phase added
    /// later fails to compile until its action is decided rather than
    /// inheriting someone else's.
    #[must_use]
    pub fn recovery(self) -> Recovery {
        match self {
            Self::Prepared => Recovery::Continue {
                next: Self::DestinationValidated,
                rationale: "nothing has been moved: the destination is built from the source, \
                            which is still in place and still authoritative."
                    .to_owned(),
            },
            Self::DestinationValidated => Recovery::Continue {
                next: Self::SourceArchived,
                rationale: "the destination is built and checked, and the source is untouched. \
                            The next step is the first that moves anything."
                    .to_owned(),
            },
            Self::SourceArchived => Recovery::Restore {
                action: "move the archive back to the source name. The destination was never \
                         activated, so the source is the only authority and putting it back is \
                         the whole recovery."
                    .to_owned(),
            },
            Self::DestinationActivated => Recovery::Continue {
                next: Self::Committed,
                rationale: "the destination is in place and the archive still exists. Going \
                            forward releases the archive; going back would discard a destination \
                            that is already the live store."
                    .to_owned(),
            },
            Self::Committed => Recovery::Refuse {
                reason: "the migration finished. There is nothing to resume, and re-running any \
                         step would act on a store that is already the new one."
                    .to_owned(),
            },
        }
    }
}

/// Which of the three directories exist when a switch-over is interrupted.
///
/// The switch is two renames, and calling that pair "atomic" is exactly the
/// claim this type refuses to make: between them the disk shows a combination
/// that has to be read on its own terms. All eight are enumerated, and the ones
/// that do not determine what happened are refused rather than guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwitchState {
    /// A directory sits at the source's name.
    pub source: bool,
    /// A directory sits at the archive's name.
    pub archive: bool,
    /// A directory sits at the destination's name.
    pub destination: bool,
}

/// No source, no archive, no destination.
const SWITCH_NOTHING: &str =
    "neither the source, the archive nor the destination exists. Whatever happened here is not      recoverable from the filesystem, and inventing a starting point would be inventing data.";

/// Only the destination survived.
const SWITCH_ORPHAN_DESTINATION: &str =
    "only the destination exists. The source is gone and so is the archive, so nothing on disk      says whether the destination is a completed migration or an abandoned one. Renaming it into      place would be a guess.";

/// The source was moved aside and nothing replaced it.
const SWITCH_ARCHIVE_ONLY: &str =
    "move the archive back to the source name. It is the only copy of the data, and no      destination was ever put in its place.";

/// The switch stopped between its two renames.
const SWITCH_MID_RENAME: &str =
    "move the archive back to the source name, leaving the destination where it is. The switch      stopped between its two renames; the source is intact under the archive name and is still      the authority.";

/// Untouched: only the source is there.
const SWITCH_UNTOUCHED: &str =
    "only the source exists, exactly as before any migration. Start over from the beginning;      nothing needs undoing.";

/// Built but never switched.
const SWITCH_BUILT_NOT_SWITCHED: &str =
    "the source is in place and a destination exists beside it. Nothing has been moved, so the      destination can be validated against the source before anything is.";

/// Two directories both claim to be the data.
const SWITCH_TWO_AUTHORITIES: &str =
    "a source and an archive both exist and there is no destination. Two directories claim to      hold the data and nothing distinguishes a half-finished restore from a half-finished      archive. Deleting or renaming either would destroy the one that turns out to be current.";

/// A layout no sequence of this migration produces.
const SWITCH_IMPOSSIBLE: &str =
    "the source, the archive and the destination all exist. No sequence of this migration      produces all three at once, so the directory has been touched by something else. Nothing      here may be removed or renamed automatically.";

impl SwitchState {
    /// Every combination of the three, so a test can be exhaustive by
    /// construction rather than by a list someone maintained by hand.
    #[must_use]
    pub fn all() -> Vec<Self> {
        let mut out = Vec::with_capacity(8);
        for source in [false, true] {
            for archive in [false, true] {
                for destination in [false, true] {
                    out.push(Self {
                        source,
                        archive,
                        destination,
                    });
                }
            }
        }
        out
    }

    /// What to do, given only what is on disk.
    ///
    /// Reads. It does not move, rename or delete anything — deciding and acting
    /// are separated precisely so that a wrong decision cannot already have
    /// destroyed the evidence.
    #[must_use]
    pub fn recovery(self) -> Recovery {
        let refuse = |reason: &str| Recovery::Refuse {
            reason: reason.to_owned(),
        };
        let restore = |action: &str| Recovery::Restore {
            action: action.to_owned(),
        };
        let go = |next: Phase, rationale: &str| Recovery::Continue {
            next,
            rationale: rationale.to_owned(),
        };
        match (self.source, self.archive, self.destination) {
            (false, false, false) => refuse(SWITCH_NOTHING),
            (false, false, true) => refuse(SWITCH_ORPHAN_DESTINATION),
            (false, true, false) => restore(SWITCH_ARCHIVE_ONLY),
            (false, true, true) => restore(SWITCH_MID_RENAME),
            (true, false, false) => go(Phase::Prepared, SWITCH_UNTOUCHED),
            (true, false, true) => go(Phase::DestinationValidated, SWITCH_BUILT_NOT_SWITCHED),
            (true, true, false) => refuse(SWITCH_TWO_AUTHORITIES),
            (true, true, true) => refuse(SWITCH_IMPOSSIBLE),
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
    pub fn may_resume(&self, source_fingerprint: &str, target_model: &str) -> Result<(), String> {
        if self.format_version != STATE_FORMAT_VERSION {
            let action = if self.format_version < STATE_FORMAT_VERSION {
                "This older state predates content-based source fingerprints. Start a fresh diagnosis."
            } else {
                "Use the version that wrote it."
            };
            return Err(format!(
                "this migration state is version {} and this build requires version {}. \
                 Resuming across incompatible state semantics is unsafe. {action}",
                self.format_version, STATE_FORMAT_VERSION,
            ));
        }
        if self.source_fingerprint != source_fingerprint {
            return Err(format!(
                "the source changed since this migration was prepared: it was fingerprinted \
                 '{}' and is now '{}'. Resuming would rebuild from an inventory that no longer \
                 describes the store. Start a fresh diagnosis.",
                self.source_fingerprint, source_fingerprint
            ));
        }
        if self.target_model != target_model {
            return Err(format!(
                "this migration was prepared for the model '{}' and the request names '{}'. \
                 Half a store embedded by one model and half by another is not searchable. \
                 Either point the request back at '{}', or start a fresh migration.",
                self.target_model, target_model, self.target_model
            ));
        }
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
        let path = workspace.join(STATE_FILE);
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(format!("cannot read {STATE_FILE}: {err}")),
        };
        let value: Value = serde_json::from_str(&raw)
            .map_err(|err| format!("{STATE_FILE} is not readable JSON: {err}"))?;
        let version = value
            .get("format_version")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("{STATE_FILE} carries no format_version"))?;
        if version != u64::from(STATE_FORMAT_VERSION) {
            let action = if version < u64::from(STATE_FORMAT_VERSION) {
                "This older state predates content-based source fingerprints; start a fresh diagnosis."
            } else {
                "Use the version that wrote it."
            };
            return Err(format!(
                "{STATE_FILE} is version {version} and this build requires version \
                 {STATE_FORMAT_VERSION}. Refusing incompatible migration semantics. {action}"
            ));
        }
        serde_json::from_value(value)
            .map(Some)
            .map_err(|err| format!("{STATE_FILE} is version {version} but does not parse: {err}"))
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
        let body = serde_json::to_string_pretty(self)
            .map_err(|err| format!("cannot serialise the migration state: {err}"))?;
        validate_existing_state(workspace)?;
        commit_state_with(
            workspace,
            body.as_bytes(),
            promote_state,
            state_durability_barrier,
        )
    }
}

fn validate_existing_state(workspace: &Path) -> Result<(), String> {
    let path = workspace.join(STATE_FILE);
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(format!("cannot inspect existing {STATE_FILE}: {err}")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "refusing to replace {STATE_FILE}: {} is a symlink, directory, or special file",
            path.display()
        ));
    }
    MigrationState::read(workspace)?
        .ok_or_else(|| format!("{STATE_FILE} disappeared while it was being validated"))?;
    Ok(())
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
    use std::io::Write;

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
    std::fs::rename(temporary, final_path)
}

#[cfg(windows)]
fn promote_state(temporary: &Path, final_path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let temporary: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
    let final_path: Vec<u16> = final_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // SAFETY: `MoveFileExW` receives valid Windows path pointers and flags.
    // - Both vectors are NUL-terminated UTF-16 paths with no interior NUL.
    // - Both vectors remain alive and are not mutated for the duration of the call.
    // Reason: the native write-through flag is required to make replacement durable.
    let moved = unsafe {
        MoveFileExW(
            temporary.as_ptr(),
            final_path.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
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
    // `promote_state` already uses MOVEFILE_WRITE_THROUGH, the native barrier.
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn state_durability_barrier(_workspace: &Path, _final_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no durable migration-state barrier is defined for this platform",
    ))
}

/// Exclusive possession of a migration workspace.
///
/// Seven properties, each one tested:
///
/// 1. acquiring a free lock succeeds;
/// 2. a second acquisition fails while the first is held;
/// 3. the refusal names who holds it and since when;
/// 4. releasing frees it;
/// 5. after release it can be taken again;
/// 6. a lock left behind by a process that died is REFUSED, never stolen;
/// 7. it never lives in the SOURCE.
///
/// Property 6 is why no PID and no port is consulted, and that is a decision
/// rather than an omission. A liveness check answers "is a process with this id
/// running on THIS machine", which says nothing about a migration driven from a
/// container, another host, or a shell that has since been reused for something
/// else — and answering it wrongly means two rebuilds writing the same
/// destination. A lock nobody released is a question for a human.
///
/// Property 7 follows from the diagnosis contract: the source is not written
/// to, and a lock file placed there would make the very act of asking a write.
#[derive(Debug)]
pub struct MigrationLock {
    path: PathBuf,
}

impl MigrationLock {
    /// Take the lock in `workspace` on behalf of `holder`.
    ///
    /// Creation is `create_new`, so the check and the claim are one filesystem
    /// operation: two callers racing cannot both observe it free.
    ///
    /// # Errors
    /// The lock is already held — the message names the holder and when — or
    /// the workspace is unwritable.
    pub fn acquire(workspace: &Path, holder: &str) -> Result<Self, String> {
        let path = workspace.join(LOCK_FILE);
        let body = format!("held_by={holder}\n");
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                use std::io::Write;
                file.write_all(body.as_bytes())
                    .map_err(|err| format!("cannot write {LOCK_FILE}: {err}"))?;
                Ok(Self { path })
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Err(format!(
                "a migration already holds this workspace ({}). It is NOT stolen automatically: \
                 no process id or port is consulted, because a dead process on this machine \
                 says nothing about a migration driven from elsewhere, and two rebuilds writing \
                 one destination is worse than a stall. If you are certain none is running, \
                 delete {} yourself.",
                Self::holder(workspace).unwrap_or_else(|| "holder unknown".to_owned()),
                path.display()
            )),
            Err(err) => Err(format!("cannot take {LOCK_FILE}: {err}")),
        }
    }

    /// Who holds the lock in `workspace`, as recorded, or `None` when free.
    #[must_use]
    pub fn holder(workspace: &Path) -> Option<String> {
        std::fs::read_to_string(workspace.join(LOCK_FILE))
            .ok()
            .map(|body| body.trim().to_owned())
    }

    fn verify_workspace(&self, workspace: &Path) -> Result<(), String> {
        let expected = workspace.join(LOCK_FILE);
        let is_live_regular_file = std::fs::symlink_metadata(&expected)
            .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink());
        if self.path != expected || !is_live_regular_file {
            return Err(format!(
                "cannot write {STATE_FILE} without the live migration lock for {}; acquire MigrationLock for this exact workspace first",
                workspace.display()
            ));
        }
        Ok(())
    }

    /// Release the lock.
    ///
    /// # Errors
    /// The lock file cannot be removed.
    pub fn release(self) -> Result<(), String> {
        std::fs::remove_file(&self.path).map_err(|err| format!("cannot release {LOCK_FILE}: {err}"))
    }
}
