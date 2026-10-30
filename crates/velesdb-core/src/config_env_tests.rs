//! Tests for the `VELESDB_*` environment-variable mapping (issue #2185).
//!
//! `VelesConfig::env_key_to_config_path` is the whole of the logic, and it is
//! a pure function, so these assert on it directly rather than on process
//! environment. That is not only convenience: `std::env::set_var` is global to
//! the process, so an end-to-end test would race any other test reading config
//! and would pass or fail depending on the thread count.
//!
//! What is being protected is the documented table in
//! `docs/guides/CONFIGURATION.md`. Before #2185 not one of its engine rows
//! reached its field — `.split("_")` nested at every underscore and
//! `.lowercase(false)` left the key uppercase — so each case below is a row
//! that used to silently do nothing.

use crate::config::VelesConfig;
use figment::value::UncasedStr;

/// Maps a variable name as it appears **after** figment strips the
/// `VELESDB_` prefix.
fn path_of(stripped: &str) -> String {
    VelesConfig::env_key_to_config_path(UncasedStr::new(stripped))
        .as_str()
        .to_string()
}

// -------------------------------------------------------------------------
// The documented rows
// -------------------------------------------------------------------------

#[test]
fn test_single_token_field_reaches_its_section() {
    // The case that proves `.lowercase(false)` was fatal on its own: this
    // name has no underscore for `.split("_")` to mangle, and it still missed.
    assert_eq!(path_of("HNSW_M"), "hnsw.m");
}

#[test]
fn test_multi_word_field_is_not_split_at_every_underscore() {
    // `hnsw.ef.construction` — the `.split("_")` failure — addresses nothing.
    assert_eq!(path_of("HNSW_EF_CONSTRUCTION"), "hnsw.ef_construction");
    assert_eq!(path_of("LIMITS_MAX_COLLECTIONS"), "limits.max_collections");
    assert_eq!(
        path_of("SEARCH_QUERY_TIMEOUT_MS"),
        "search.query_timeout_ms"
    );
    assert_eq!(
        path_of("QUANTIZATION_RERANK_MULTIPLIER"),
        "quantization.rerank_multiplier"
    );
}

#[test]
fn test_lowercase_and_mixed_case_names_map_the_same() {
    // Figment matches the key against the serde field name, which is
    // lowercase; the variable's own casing must not decide whether it works.
    for name in [
        "HNSW_EF_CONSTRUCTION",
        "hnsw_ef_construction",
        "Hnsw_Ef_Construction",
    ] {
        assert_eq!(path_of(name), "hnsw.ef_construction", "for {name}");
    }
}

#[test]
fn test_every_section_of_the_struct_is_addressable() {
    // A section missing from `ENV_SECTIONS` would fall through as a flat key
    // and silently match nothing — the exact shape of this bug.
    for (section, field) in [
        ("search", "max_results"),
        ("hnsw", "max_layers"),
        ("storage", "storage_mode"),
        ("limits", "max_payload_size"),
        ("server", "max_body_size"),
        ("logging", "level"),
        ("quantization", "enabled"),
    ] {
        let var = format!("{section}_{field}").to_ascii_uppercase();
        assert_eq!(path_of(&var), format!("{section}.{field}"), "for {var}");
    }
}

#[test]
fn test_a_removed_section_is_still_split_at_its_own_boundary() {
    // `[wal_batch]` left the struct in 7.0.0 (#2174), but a 6.x deployment can
    // still export its variables. Splitting after `wal_batch` keeps them on the
    // removed path, where the loader names them in its removal warning; split
    // nowhere, they would flatten into `wal_batch_enabled`, a key nothing
    // reports. It is also the one section whose *name* contains an underscore.
    assert_eq!(
        path_of("WAL_BATCH_MAX_BATCH_SIZE"),
        "wal_batch.max_batch_size"
    );
    assert_eq!(path_of("WAL_BATCH_ENABLED"), "wal_batch.enabled");
}

// -------------------------------------------------------------------------
// What must stay inert
// -------------------------------------------------------------------------

#[test]
fn test_names_outside_any_section_pass_through_unsplit() {
    // These are read elsewhere (the CLI's own config path, the update check)
    // or belong to `velesdb-server`'s transport config. They matched nothing
    // in `VelesConfig` before and must keep matching nothing: inventing a
    // nesting for them is how an unrelated variable starts steering the
    // engine.
    for name in ["CONFIG", "NO_UPDATE_CHECK", "HOST", "PORT", "API_KEYS"] {
        let mapped = path_of(name);
        assert!(
            !mapped.contains('.'),
            "{name} must not be given a section, got {mapped}"
        );
    }
}

#[test]
fn test_a_section_name_with_no_field_is_not_split() {
    // `VELESDB_STORAGE` addresses the table itself, not a field in it;
    // emitting `storage.` would be a malformed path.
    assert_eq!(path_of("STORAGE"), "storage");
    assert_eq!(path_of("HNSW"), "hnsw");
}

#[test]
fn test_a_name_merely_starting_with_a_section_is_not_split() {
    // `searching_for` starts with `search` but the next character is not the
    // separator, so it is not a `[search]` field. Requiring the underscore is
    // what keeps this from becoming `search.ing_for`.
    assert_eq!(path_of("SEARCHING_FOR"), "searching_for");
    assert_eq!(path_of("SERVERLESS_MODE"), "serverless_mode");
}

// -------------------------------------------------------------------------
// The engine-only loader's environment filter
// -------------------------------------------------------------------------

/// `load_from_path_engine_only` reads only the variables this accepts. A
/// hosting shell's `[server]`/`[logging]` variables must not reach this
/// struct's same-named fields, where they were validated against rules for
/// values the engine never applies (`VELESDB_SERVER_PORT=80` refused the
/// server's startup with `server.port must be >= 1024`).
#[test]
fn test_engine_only_env_filter_keeps_engine_sections_and_drops_the_shells() {
    for engine in [
        "LIMITS_MAX_COLLECTIONS",
        "HNSW_EF_CONSTRUCTION",
        "SEARCH_DEFAULT_MODE",
    ] {
        assert!(
            VelesConfig::is_engine_path(&path_of(engine)),
            "{engine} must reach the engine"
        );
    }
    for shell in [
        "SERVER_PORT",
        "SERVER_WORKERS",
        "LOGGING_LEVEL",
        "HOST",
        "CONFIG",
    ] {
        assert!(
            !VelesConfig::is_engine_path(&path_of(shell)),
            "{shell} belongs to the hosting shell"
        );
    }
}

/// A removed section stays visible to the loader, so its warning fires.
#[test]
fn test_engine_only_env_filter_keeps_removed_sections() {
    assert!(
        crate::config_removed::removed_sections().next().is_some(),
        "no removed section to check against"
    );
    for removed in crate::config_removed::removed_sections() {
        let name = format!("{}_SOME_FIELD", removed.to_ascii_uppercase());
        assert!(VelesConfig::is_engine_path(&path_of(&name)), "{name}");
    }
}
