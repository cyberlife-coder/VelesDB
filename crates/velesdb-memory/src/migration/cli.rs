//! The operator surface for a rebuild (#1815).
//!
//! # Why a command and not an MCP tool
//!
//! A migration is long, mutates a store, has to show progress and has to be
//! interruptible and resumable. None of that survives being tied to an MCP
//! session's lifetime, and #1727 records a client defect that makes a long or
//! destructive tool call particularly unsuitable. So the first surface is a
//! subcommand, the engine stays a library, and no mutating MCP tool is added.
//!
//! # What this phase does and does not do
//!
//! `--dry-run` only. It reads, it decides a regime, it prints what it found and
//! what would happen. It writes nothing, takes no lock, and may run while the
//! daemon holds the store: [`diagnose`](crate::migration::diagnose) never opens
//! the live source, it inspects a verified copy.
//!
//! Any invocation that is not a dry run is REFUSED with the reason, rather than
//! accepted and quietly doing nothing. An operator who types
//! `migrate-embeddings --strategy auto` is asking for a migration; answering
//! with silence and exit 0 would be the worst of the available behaviours.

use super::{diagnose, DiagnosisReport, Strategy, TargetContract};
use std::path::{Path, PathBuf};

/// What the operator asked the command to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrateOptions {
    /// The store to rebuild. `None` means "wherever the daemon would look".
    pub store: Option<PathBuf>,
    /// Where the rebuilt store would be written. Inspected, never created.
    pub destination: Option<PathBuf>,
    /// Where the diagnosis stages its verified copy. Needs room for the store.
    pub scratch: Option<PathBuf>,
    /// The regime, `auto` unless stated.
    pub strategy: Strategy,
    /// Whether this run is a dry run. The only supported value today is `true`.
    pub dry_run: bool,
}

impl Default for MigrateOptions {
    fn default() -> Self {
        Self {
            store: None,
            destination: None,
            scratch: None,
            strategy: Strategy::Auto,
            dry_run: false,
        }
    }
}

/// What an operator must be told after a COMPLETED migration.
///
/// This text replaced `NOT_YET_SWITCHABLE` when C3 wired validation and the
/// switch. Two things in it are load-bearing. The daemon holds its store as
/// an in-memory handle taken at startup and never refreshed, so a daemon that
/// was running against the old store keeps serving the old data until it is
/// RESTARTED — a message that failed to say so would let an operator migrate
/// perfectly and then wonder where the new vectors went. And the journal is
/// left in place deliberately: it is the evidence of what happened, and its
/// removal is the operator's call, not this command's.
const MIGRATION_COMPLETE: &str =
    "the migration is complete: the rebuilt store now sits at the source's \
     path, its provenance stamp names the target embedder, and the archived \
     old store has been freed. RESTART the daemon — it holds its store as a \
     handle taken at startup, and until it restarts it keeps serving the old \
     data from memory. The .migration-journal directory beside the (now \
     empty) destination path is the journal of what happened; it may be \
     removed once you no longer want the evidence.";

/// Parse `migrate-embeddings`' flags.
///
/// Hand-rolled for the same reason as `--version` and `compile-stdin` in the
/// binary: this crate ships one binary and does not carry `clap` for it.
///
/// # Errors
/// A message naming the offending flag when it is unknown, when a value is
/// missing, or when `--strategy` is not one of the three arbitrated regimes.
pub fn parse(args: &[String]) -> Result<MigrateOptions, String> {
    let mut options = MigrateOptions::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        if flag == "--dry-run" {
            options.dry_run = true;
            index += 1;
            continue;
        }
        let value = value_for(args, index, flag)?;
        apply_valued_flag(&mut options, flag, value)?;
        index += 2;
    }
    Ok(options)
}

/// The value following `flag`, or a message naming what is missing.
fn value_for<'a>(args: &'a [String], index: usize, flag: &str) -> Result<&'a String, String> {
    args.get(index + 1)
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn apply_valued_flag(
    options: &mut MigrateOptions,
    flag: &str,
    value: &String,
) -> Result<(), String> {
    match flag {
        "--store" => options.store = Some(PathBuf::from(value)),
        "--destination" => options.destination = Some(PathBuf::from(value)),
        "--scratch" => options.scratch = Some(PathBuf::from(value)),
        "--strategy" => options.strategy = Strategy::parse(value)?,
        other => return Err(format!("unknown migrate-embeddings flag {other:?}")),
    }
    Ok(())
}

/// Diagnose the store under `options` against `target`, and render the result.
///
/// Reads only. The source is never opened by
/// [`velesdb_core::Database::open`] — a verified copy under the scratch parent
/// is — so this runs against a store the daemon still holds.
///
/// # Errors
/// [`crate::MemoryError`] when the store cannot be read, copied or walked, and
/// a plain message when the invocation is not a dry run.
pub fn dry_run(
    store: &Path,
    scratch_parent: &Path,
    target: &TargetContract,
    destination: Option<&Path>,
) -> Result<DiagnosisReport, crate::MemoryError> {
    diagnose(store, scratch_parent, target, destination)
}

/// Require the flag a non-dry-run cannot proceed without.
///
/// The rebuild writes somewhere, and "wherever seems sensible" is not an
/// answer when the somewhere will later be renamed over the live store: the
/// operator names the destination, or the run does not start.
///
/// # Errors
/// A message naming the missing flag when `--destination` was not given.
pub fn require_destination(options: &MigrateOptions) -> Result<PathBuf, String> {
    options.destination.clone().ok_or_else(|| {
        "a non-dry-run migrate-embeddings rebuilds into a destination you name: \
         pass --destination <dir> (an empty or not-yet-existing directory on \
         the store's filesystem), or --dry-run to only diagnose"
            .to_owned()
    })
}

/// Render a report for an operator: what is here, what would happen, and what
/// still blocks it.
///
/// The regime comes FIRST and on its own line. Everything else is context for
/// it, and burying the one decision in a table of counts is how an operator
/// ends up acting on the wrong one.
#[must_use]
pub fn render(report: &DiagnosisReport) -> String {
    let guidance = report
        .resolution
        .guidance()
        .map_or_else(String::new, |next| format!("{next}\n\n"));
    format!(
        "{}\n\n{guidance}{}{}{}",
        report.resolution.diagnostic(),
        render_identity(report),
        render_inventory(report),
        render_blockers(report),
    )
}

fn render_identity(report: &DiagnosisReport) -> String {
    let provenance = match &report.source_provenance {
        super::SourceProvenance::Known { model, dimension } => {
            format!("{model} ({dimension} dimensions)")
        }
        super::SourceProvenance::Unknown { .. } => {
            "unknown — not inferred from the width".to_owned()
        }
    };
    let source_dimension = report
        .source_dimension
        .map_or_else(|| "no shared width".to_owned(), |d| d.to_string());
    format!(
        "  store:              {}\n  \
           source provenance:  {provenance}\n  \
           source dimension:   {source_dimension}\n  \
           target model:       {} ({} dimensions)\n  \
           requested strategy: {:?}\n  \
           report format:      v{}\n\n",
        report.source_path.display(),
        report.target_model,
        report.target_dimension,
        report.requested_strategy,
        report.format_version,
    )
}

fn render_inventory(report: &DiagnosisReport) -> String {
    format!(
        "  facts:              {}\n  \
           edges:              {}\n  \
           working contexts:   {}\n  \
           facts with a TTL:   {}\n  \
           bytes on disk:      {}\n\n",
        report.facts,
        report.edges,
        report.working_contexts,
        report.ttl_summary.with_expiry,
        report.bytes_on_disk,
    )
}

fn render_blockers(report: &DiagnosisReport) -> String {
    if report.blockers.is_empty() {
        return "no outstanding blockers.\n".to_owned();
    }
    let listed = report
        .blockers
        .iter()
        .fold(String::new(), |mut acc, blocker| {
            acc.push_str("  - ");
            acc.push_str(blocker);
            acc.push('\n');
            acc
        });
    format!(
        "{} blocker(s) before a rebuild:\n{listed}",
        report.blockers.len()
    )
}

/// Whether this diagnosis leaves the command with nothing it could run.
///
/// Separate from rendering so the exit status and the text cannot drift: a
/// command that printed `REFUSE` and exited 0 would be read as success by every
/// script that wraps it.
#[must_use]
pub fn refuses(report: &DiagnosisReport) -> bool {
    !report.resolution.runs()
}

/// Re-exported so the binary can print the completion without duplicating it.
#[must_use]
pub fn migration_complete_notice() -> &'static str {
    MIGRATION_COMPLETE
}

/// The default scratch parent: the directory the store itself sits in.
///
/// This used to be `std::env::temp_dir()`, and Codacy's finding on it
/// ("`temp_dir` should not be used for security operations") was right for a
/// reason it did not name. Secrecy is already handled — the copy lands in a
/// directory created `0o700` with an owner token, one level down — but the
/// wrong VOLUME is not: the diagnosis copies the whole store, temp
/// filesystems are routinely small and sometimes RAM-backed, and the doc
/// comment that used to sit here named that failure mode while the code
/// shipped it as the default anyway. The store's parent is on the store's
/// volume by construction, so room for a copy of the store is at least
/// plausible there — and the staging check still measures it rather than
/// assuming it.
///
/// No silent fallback: a store with no usable parent is an error naming
/// `--scratch`, never a quiet switch to a different volume.
///
/// # Errors
/// The store path has no non-empty parent to stage beside.
pub fn default_scratch_parent(store: &Path) -> Result<PathBuf, String> {
    match store.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => Ok(parent.to_path_buf()),
        _ => Err(format!(
            "cannot derive a scratch parent beside {}: pass --scratch <dir>. The diagnosis \
             copies the whole store there, so a directory on the store's own volume is best",
            store.display()
        )),
    }
}
