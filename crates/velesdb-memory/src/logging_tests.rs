use super::filter_from_raw;

#[test]
fn unset_or_blank_yields_no_filter() {
    // The "silent by default" half of the contract: no filter, so
    // `init_from_env` installs no subscriber — the daemon must behave
    // byte-for-byte as before.
    assert!(matches!(filter_from_raw(None), Ok(None)));
    assert!(matches!(filter_from_raw(Some("")), Ok(None)));
    assert!(matches!(filter_from_raw(Some("   ")), Ok(None)));
}

#[test]
fn a_directive_list_yields_a_filter() {
    // Positive control for the silence test above: a function that
    // answered `None` to everything would pass it vacuously.
    assert!(matches!(filter_from_raw(Some("info")), Ok(Some(_))));
    assert!(matches!(
        filter_from_raw(Some("info,rmcp=debug")),
        Ok(Some(_))
    ));
}

#[test]
fn the_incident_preset_parses() {
    // The preset is dead the day it stops parsing — the daemon would
    // refuse to boot with it, which is exactly when an operator needs it.
    assert!(matches!(
        filter_from_raw(Some(super::INCIDENT_PRESET)),
        Ok(Some(_))
    ));
}

#[test]
fn the_installer_ships_the_incident_preset_verbatim() {
    // The plist is written by a shell script that cannot read this
    // constant, so the two CAN drift — and a drifted installer would
    // deploy a daemon logging either nothing or, worse, a payload-leaking
    // filter. Verbatim containment is the whole check.
    //
    // Read at runtime, not `include_str!`: `scripts/` is not packaged
    // into the published .crate, so a compile-time include would make
    // `cargo test` unbuildable from the .crate or a vendored source.
    // Outside the repository there is no installer to drift against, so
    // absence is a genuine pass — in the repository (and its CI, where
    // this guard matters) the file always exists.
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scripts/install-memory-daemon.sh"
    );
    let Ok(installer) = std::fs::read_to_string(path) else {
        return;
    };
    assert!(
        installer.contains(super::INCIDENT_PRESET),
        "scripts/install-memory-daemon.sh must wire VELESDB_MEMORY_LOG to the \
         incident preset exactly as src/logging.rs declares it"
    );
}

#[test]
fn an_unparseable_value_is_refused_with_the_var_name() {
    // Refusal proven, not assumed: an operator who set a broken filter
    // asked for logs and must not silently get silence instead.
    let err =
        filter_from_raw(Some("velesdb=notalevel")).expect_err("an invalid level must be refused");
    assert!(
        err.contains("VELESDB_MEMORY_LOG"),
        "the message must name the var to fix, got: {err}"
    );
}
