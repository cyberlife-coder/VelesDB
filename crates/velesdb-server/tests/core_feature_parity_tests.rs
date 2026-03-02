use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use toml::Value;

fn parse_features(manifest_path: &Path) -> BTreeMap<String, Vec<String>> {
    let manifest = fs::read_to_string(manifest_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", manifest_path.display()));

    let parsed: Value = manifest
        .parse()
        .unwrap_or_else(|e| panic!("failed to parse {} as TOML: {e}", manifest_path.display()));

    let features = parsed
        .get("features")
        .and_then(Value::as_table)
        .unwrap_or_else(|| panic!("missing [features] in {}", manifest_path.display()));

    features
        .iter()
        .map(|(feature, entries)| {
            let values = entries
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            (feature.clone(), values)
        })
        .collect()
}

#[test]
fn server_exposes_all_core_features() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let core_manifest = workspace_root.join("crates/velesdb-core/Cargo.toml");
    let server_manifest = workspace_root.join("crates/velesdb-server/Cargo.toml");

    let core_features = parse_features(&core_manifest);
    let server_features = parse_features(&server_manifest);

    let missing: Vec<_> = core_features
        .keys()
        .filter(|feature| !server_features.contains_key(*feature))
        .cloned()
        .collect();

    assert!(
        missing.is_empty(),
        "velesdb-server is missing core features: {missing:?}"
    );

    for feature in core_features.keys() {
        if let Some(entries) = server_features.get(feature) {
            let expected = format!("velesdb-core/{feature}");
            assert!(
                entries.iter().any(|entry| entry == &expected),
                "server feature `{feature}` must forward to `{expected}`"
            );
        }
    }
}

#[test]
fn server_default_feature_matches_core_default() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let core_manifest = workspace_root.join("crates/velesdb-core/Cargo.toml");
    let server_manifest = workspace_root.join("crates/velesdb-server/Cargo.toml");

    let core_features = parse_features(&core_manifest);
    let server_features = parse_features(&server_manifest);

    let core_default: BTreeSet<_> = core_features
        .get("default")
        .into_iter()
        .flatten()
        .cloned()
        .collect();

    let server_default: BTreeSet<_> = server_features
        .get("default")
        .into_iter()
        .flatten()
        .cloned()
        .collect();

    let forwards_core_default_directly = server_default.contains("velesdb-core/default");

    if !forwards_core_default_directly {
        for core_default_feature in core_default {
            let expected = format!("velesdb-core/{core_default_feature}");
            assert!(
                server_default.contains(&expected),
                "server default feature set must include `{expected}`"
            );
        }
    }
}
