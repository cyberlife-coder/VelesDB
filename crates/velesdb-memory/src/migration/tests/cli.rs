//! What the operator surface accepts, refuses, and says (#1815).
//!
//! The module this exercises is the first production caller the `migration`
//! module has ever had. Before it, ~7 000 lines of diagnosis, enumeration,
//! re-insertion, journal and lock were reachable only from tests — which is the
//! finding that opened #1815.

use super::diagnosis::{diagnose_as, TARGET_DIM, TARGET_MODEL};
use super::*;

fn args(raw: &[&str]) -> Vec<String> {
    raw.iter().map(|s| (*s).to_owned()).collect()
}

// ---------------------------------------------------------------------------
// WHAT THE COMMAND LINE ACCEPTS
// ---------------------------------------------------------------------------

#[test]
fn the_default_is_auto_and_not_a_dry_run() {
    let options = parse_migrate_args(&[]).expect("no flags is a valid invocation");

    assert_eq!(options.strategy, Strategy::Auto);
    assert!(
        !options.dry_run,
        "a dry run must be asked for; defaulting to it would make the flag \
         meaningless and leave the non-dry path unreachable"
    );
}

#[test]
fn every_flag_is_parsed_and_an_unknown_one_is_named() {
    let options = parse_migrate_args(&args(&[
        "--dry-run",
        "--store",
        "/tmp/store",
        "--destination",
        "/tmp/dest",
        "--scratch",
        "/tmp/scratch",
        "--strategy",
        "reembed",
    ]))
    .expect("a complete invocation");

    assert!(options.dry_run);
    assert_eq!(
        options.store.as_deref(),
        Some(std::path::Path::new("/tmp/store"))
    );
    assert_eq!(
        options.destination.as_deref(),
        Some(std::path::Path::new("/tmp/dest"))
    );
    assert_eq!(
        options.scratch.as_deref(),
        Some(std::path::Path::new("/tmp/scratch"))
    );
    assert_eq!(options.strategy, Strategy::Reembed);

    let unknown =
        parse_migrate_args(&args(&["--nope", "x"])).expect_err("an unknown flag must be refused");
    assert!(unknown.contains("--nope"), "{unknown}");
}

#[test]
fn a_flag_without_its_value_names_the_flag_rather_than_the_position() {
    for flag in ["--store", "--destination", "--scratch", "--strategy"] {
        let message =
            parse_migrate_args(&args(&[flag])).expect_err("a valueless flag must be refused");

        assert!(
            message.contains(flag) && message.contains("requires a value"),
            "{flag} should name itself: {message}"
        );
    }
}

#[test]
fn force_reuse_is_refused_at_the_command_line_and_not_only_in_the_rule() {
    // An operator meets the command line, not `Strategy::parse`. A rule nobody
    // can reach refuses nothing.
    let message = parse_migrate_args(&args(&["--strategy", "force-reuse"]))
        .expect_err("force-reuse must not parse");

    assert!(
        message.contains("does not exist, and not by oversight")
            && message.contains("--strategy reembed"),
        "{message}"
    );
}

#[test]
fn anything_that_is_not_a_dry_run_is_refused_rather_than_silently_doing_nothing() {
    let asked_for_a_migration = parse_migrate_args(&args(&["--strategy", "auto"])).expect("flags");

    let refusal = require_dry_run(&asked_for_a_migration)
        .expect_err("a non-dry-run invocation must be refused in this version");

    assert!(
        refusal.contains("--dry-run") && refusal.contains("#1762"),
        "the refusal must name the supported mode and where the rest is tracked: {refusal}"
    );

    require_dry_run(&parse_migrate_args(&args(&["--dry-run"])).expect("flags"))
        .expect("a dry run must be accepted");
}

// ---------------------------------------------------------------------------
// WHAT THE OPERATOR READS, AND WHAT THE SHELL READS
// ---------------------------------------------------------------------------

#[test]
fn the_dry_run_reads_a_real_store_and_leads_with_the_regime() {
    let (dir, _ttl) = seeded();

    let report = diagnose_as(dir.path(), Strategy::Auto, TARGET_MODEL, TARGET_DIM);
    let rendered = render(&report);

    assert!(
        rendered.starts_with("REEMBED: source provenance is unknown"),
        "the one decision an operator acts on must come first: {rendered}"
    );
    assert!(
        rendered.contains("not inferred from the width"),
        "the provenance line must say the model was not guessed: {rendered}"
    );
    assert!(
        rendered.contains(&format!("facts:              {}", SEEDED + 1)),
        "the inventory must be the store's, not a template: {rendered}"
    );
    assert!(
        rendered.contains("blocker(s) before a rebuild:"),
        "outstanding blockers must be shown, not summarised away: {rendered}"
    );
    assert!(!refuses(&report), "re-embedding is a decision that runs");
}

#[test]
fn a_refusal_renders_its_way_out_and_is_reportable_as_a_refusal() {
    let (dir, _ttl) = seeded();

    let report = diagnose_as(dir.path(), Strategy::Reuse, TARGET_MODEL, TARGET_DIM);
    let rendered = render(&report);

    assert!(
        rendered.starts_with("REFUSE: reuse was requested, but source provenance is unknown"),
        "{rendered}"
    );
    assert!(
        rendered.contains("--strategy reembed"),
        "a refusal must render its way out: {rendered}"
    );
    assert!(
        refuses(&report),
        "a refusal must be reportable as one, or a wrapping script reads exit 0 as success"
    );
}

#[test]
fn the_dry_run_leaves_the_store_byte_for_byte_as_it_found_it() {
    // The command is read-only, and "read-only" is a claim until something
    // checks it. The daemon may be holding this store while it runs.
    let (dir, _ttl) = seeded();
    let before = super::diagnosis::tree(dir.path());

    let staging = tempfile::tempdir().expect("staging");
    let report = dry_run(
        dir.path(),
        staging.path(),
        &TargetContract::automatic(TARGET_MODEL, TARGET_DIM),
        None,
    )
    .expect("dry run");

    assert!(report.facts > 0, "the fixture must not be empty");
    assert_eq!(
        super::diagnosis::tree(dir.path()),
        before,
        "a dry run must not change one byte of the store it inspects"
    );
}

#[test]
fn the_default_scratch_parent_is_the_store_s_own_volume_not_the_temp_dir() {
    let (dir, _ttl) = seeded();

    let parent = default_scratch_parent(dir.path()).expect("a real store has a parent");

    assert_eq!(
        parent,
        dir.path().parent().expect("tempdir has a parent"),
        "the diagnosis copies the WHOLE store; staging beside it is on the \
         store's volume by construction, where a temp filesystem sized for \
         small files is exactly where a real store would fail"
    );

    // The default must be USABLE, not merely computed: a dry run actually
    // stages there and completes.
    let report = dry_run(
        dir.path(),
        &parent,
        &TargetContract::automatic(TARGET_MODEL, TARGET_DIM),
        None,
    )
    .expect("a dry run staging beside the store");
    assert!(report.facts > 0, "the fixture must not be empty");

    // And a store with no usable parent is an ERROR naming the flag — never a
    // quiet switch to a different volume, which is how the old temp_dir
    // default would have resurfaced under another name.
    let rootless = default_scratch_parent(std::path::Path::new("/"))
        .expect_err("no parent to stage beside must refuse");
    assert!(rootless.contains("--scratch"), "{rootless}");
}
