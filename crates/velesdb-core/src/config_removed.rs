//! Configuration keys a release removed, kept by name only.
//!
//! `VelesConfig` does not deny unknown keys: whatever no field claims, figment
//! drops at extraction. That leniency is what lets a 6.x file keep loading
//! under 7.0.0, and it would also drop a removed key without a trace, leaving a
//! deployment that still sets `[wal_batch]` believing the line does something.
//! So every loader looks these paths up in its merged sources (the file and the
//! `VELESDB_*` environment alike) and names the ones it finds in one warning.
//!
//! Nothing here is applied, validated or range-checked. A removed key is
//! ignored whatever its value, including the `mmap_cache_mb = 0` that 6.x
//! refused at load.

use figment::Figment;

/// The release that removed every key in [`REMOVED_KEYS`].
const REMOVED_IN: &str = "7.0.0";

/// Each removed key as a TOML path, with the issue that removed it.
///
/// A path without a dot is a whole table. The engine-only filter and the
/// environment mapping both derive their extra section names from this list
/// through [`removed_sections`], so a table cannot be named here and missed
/// there.
const REMOVED_KEYS: &[(&str, u32)] = &[
    ("wal_batch", 2174),
    ("storage.data_dir", 2087),
    ("storage.mmap_cache_mb", 2087),
    ("storage.vector_alignment", 2087),
];

/// The removed top-level tables.
///
/// `VelesConfig::load_from_path_engine_only` keeps them when it drops foreign
/// tables, and `VelesConfig::env_key_to_config_path` still splits a variable
/// at their boundary, so a removed table reaches [`removed_keys_set`] from
/// every source instead of being discarded or flattened before it is seen.
pub(crate) fn removed_sections() -> impl Iterator<Item = &'static str> {
    REMOVED_KEYS
        .iter()
        .map(|&(path, _)| path)
        .filter(|path| !path.contains('.'))
}

/// The removed keys `figment` still sets, as TOML paths, in table order.
///
/// Split out of [`warn_removed_keys`] so the set is assertable: a `tracing`
/// warning is only observable from a test by capturing it.
pub(crate) fn removed_keys_set(figment: &Figment) -> Vec<&'static str> {
    REMOVED_KEYS
        .iter()
        .map(|&(path, _)| path)
        .filter(|path| figment.find_value(path).is_ok())
        .collect()
}

/// Logs one warning naming every removed key `figment` sets, and nothing when
/// it sets none.
pub(crate) fn warn_removed_keys(figment: &Figment) {
    let found = removed_keys_set(figment);
    if found.is_empty() {
        return;
    }
    let removed = REMOVED_KEYS
        .iter()
        .filter(|(path, _)| found.contains(path))
        .map(|(path, issue)| format!("{path} (#{issue})"))
        .collect::<Vec<_>>()
        .join(", ");
    tracing::warn!(
        removed,
        "these config entries were removed in {REMOVED_IN} and are ignored; \
         delete them from the file or the VELESDB_* environment"
    );
}
