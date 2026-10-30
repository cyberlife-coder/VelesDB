use super::*;

/// The compile-time guard `ALL` cannot have: only this crate can enumerate a
/// `non_exhaustive` enum, so only this crate can notice `ALL` going stale.
/// A `match` over `Self` keeps it honest — adding a category without touching
/// `ALL` fails to compile HERE (the defining crate still matches
/// exhaustively), and the assertion documents the contract for readers.
#[test]
fn all_lists_every_category() {
    let mut seen = 0usize;
    for category in ErrorCategory::ALL {
        // Exhaustive on purpose; a new variant must be added to `ALL` and to
        // this match in the same change.
        match category {
            ErrorCategory::InvalidInput
            | ErrorCategory::NotFound
            | ErrorCategory::Internal
            | ErrorCategory::Unsupported => {
                seen += 1;
            }
        }
    }
    assert_eq!(
        seen,
        ErrorCategory::ALL.len(),
        "ALL carries a duplicate or an unmatched entry"
    );
}

/// A migration failure reaches the adapters through [`MemoryError::category`],
/// so the variant decides what a client is told: an operator who runs the
/// steps out of order made a mistake, which is not a fault of the server.
#[cfg(feature = "persistence")]
mod migration_errors {
    use super::*;
    use crate::migration::MigrationError;

    fn category_of(error: MigrationError) -> ErrorCategory {
        MemoryError::from(error).category()
    }

    #[test]
    fn something_the_caller_named_that_does_not_exist_is_not_found() {
        let error = MigrationError::NotFound("collection `facts` not found".into());
        assert_eq!(category_of(error), ErrorCategory::NotFound);
    }

    #[test]
    fn a_step_run_out_of_order_or_on_unusable_input_is_the_callers_to_fix() {
        let error = MigrationError::Precondition("run the rebuild first".into());
        assert_eq!(category_of(error), ErrorCategory::InvalidInput);
    }

    #[test]
    fn a_failing_filesystem_or_a_full_disk_is_an_internal_fault() {
        let error = MigrationError::Environment("cannot flush the diagnostic copy".into());
        assert_eq!(category_of(error), ErrorCategory::Internal);
    }

    #[test]
    fn a_refused_capture_is_an_internal_fault_that_keeps_its_detail() {
        let error = crate::migration::capture("a mutation observer is already active");

        assert!(
            matches!(error, MemoryError::Migration(MigrationError::Capture(_))),
            "{error:?}"
        );
        assert_eq!(error.category(), ErrorCategory::Internal);
        assert_eq!(
            error.to_string(),
            "migration error: a mutation observer is already active"
        );
    }

    #[test]
    fn data_that_contradicts_itself_is_an_internal_fault() {
        let error = MigrationError::Integrity("the edge indexes disagree".into());
        assert_eq!(category_of(error), ErrorCategory::Internal);
    }

    /// The message used to read `storage error: [VELES-010] Query error: …`,
    /// which names a storage engine and a query that were never involved.
    #[test]
    fn the_message_is_the_cause_and_names_no_engine() {
        let text = MemoryError::from(MigrationError::Precondition(
            "the destination has not been validated".into(),
        ))
        .to_string();
        assert!(
            text.contains("the destination has not been validated"),
            "{text}"
        );
        assert!(!text.contains("storage error"), "{text}");
        assert!(!text.contains("VELES-010"), "{text}");
    }
}

/// Fitness function: the engine's `Query` error is for queries. Reusing it as
/// a carrier for this crate's own failures is what made every migration
/// mistake read as a storage fault, and it takes one `format!` to start again.
#[cfg(feature = "persistence")]
#[test]
fn no_production_code_disguises_a_domain_failure_as_a_core_query_error() {
    use std::path::Path;

    fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("test: read src dir") {
            let path = entry.expect("test: dir entry").path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "tests") {
                    continue;
                }
                rust_files(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if !stem.ends_with("_tests") && stem != "tests" {
                    out.push(path);
                }
            }
        }
    }

    let mut files = Vec::new();
    rust_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    assert!(!files.is_empty(), "the scan found no source file");

    let offenders: Vec<String> = files
        .iter()
        .filter(|path| !path.ends_with("error.rs"))
        .filter(|path| {
            std::fs::read_to_string(path)
                .expect("test: read source")
                .lines()
                .map(str::trim_start)
                .filter(|line| !line.starts_with("//"))
                .any(|line| line.contains("Error::Query("))
        })
        .map(|path| path.display().to_string())
        .collect();
    assert!(
        offenders.is_empty(),
        "these files build a core `Error::Query` for a failure that is not a query; \
         use `MigrationError` (or a variant of `MemoryError`): {offenders:#?}"
    );
}
