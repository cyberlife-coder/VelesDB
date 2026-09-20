//! Anti-drift guard for the fusion table in `docs/guides/CORE_SPARSE_AND_FUSION.md`.
//!
//! The guide named two of the six `FusionStrategy` variants, and had for long
//! enough that an adversarial audit filed it twice (#2095, finding D). A
//! hand-kept list of enum arms drifts the moment an arm is added — so this
//! derives the expected names from the enum's own source and asserts the table
//! names each one.
//!
//! It reads `fusion/strategy.rs` rather than reflecting over the type because
//! Rust has no runtime variant list for a non-`strum` enum, and adding a
//! derive to production code to satisfy a doc test would be the tail wagging
//! the dog. The parser is narrow on purpose: variants of `FusionStrategy` are
//! declared at exactly one indentation level inside one `pub enum` block, and
//! the test fails loudly if it stops finding them.

use std::path::PathBuf;

fn repo_file(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// Every variant name of `FusionStrategy`, read from its declaration.
fn declared_variants() -> Vec<String> {
    let source = std::fs::read_to_string(repo_file("crates/velesdb-core/src/fusion/strategy.rs"))
        .expect("test: fusion/strategy.rs is readable");
    let start = source
        .find("pub enum FusionStrategy")
        .expect("test: `pub enum FusionStrategy` moved; this guard needs updating");
    let mut depth = 0usize;
    let mut end = source.len();
    for (offset, ch) in source[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = start + offset;
                    break;
                }
            }
            _ => {}
        }
    }
    source[start..end]
        .lines()
        .filter_map(|line| {
            // A variant sits at four spaces and is followed by `,` or `{`.
            let rest = line.strip_prefix("    ")?;
            if rest.starts_with(' ') || rest.starts_with("//") || rest.starts_with('#') {
                return None;
            }
            let name: String = rest
                .chars()
                .take_while(char::is_ascii_alphanumeric)
                .collect();
            let tail = rest[name.len()..].trim_start();
            let declares = tail.starts_with(',') || tail.starts_with('{');
            (!name.is_empty() && name.starts_with(char::is_uppercase) && declares).then_some(name)
        })
        .collect()
}

#[test]
fn the_guide_names_every_fusion_strategy() {
    let variants = declared_variants();
    assert!(
        variants.len() >= 5,
        "parsed {variants:?} from fusion/strategy.rs — the parser broke, not the guide"
    );

    let guide = std::fs::read_to_string(repo_file("docs/guides/CORE_SPARSE_AND_FUSION.md"))
        .expect("test: the fusion guide is readable");

    let missing: Vec<&String> = variants
        .iter()
        .filter(|name| !guide.contains(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "docs/guides/CORE_SPARSE_AND_FUSION.md names {} of {} fusion strategies; \
         absent: {missing:?}. A caller reading the guide cannot ask for what it does not name.",
        variants.len() - missing.len(),
        variants.len()
    );
}

#[test]
fn the_guide_states_where_each_strategy_is_reachable() {
    // Naming a variant is not enough: `WeightedRRF` reaches only the Rust
    // embedding API, and a table that lists it without saying so would send a
    // Python or REST caller looking for a string that does not exist.
    let guide = std::fs::read_to_string(repo_file("docs/guides/CORE_SPARSE_AND_FUSION.md"))
        .expect("test: the fusion guide is readable");
    assert!(
        guide.contains("where each one is reachable"),
        "the guide lost its reachability table"
    );
    assert!(
        guide.contains("WeightedRRF") && guide.contains("embedding API"),
        "the guide names WeightedRRF without saying which surfaces can reach it"
    );
}
