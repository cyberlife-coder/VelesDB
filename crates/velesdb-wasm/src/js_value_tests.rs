//! Guard for [`super::to_js_value`] being the crate's only outbound
//! serializer (#2332).
//!
//! What the serializer emits is only observable in a JS runtime, so the
//! shapes are asserted by `tests/result_shape_web.rs` under
//! `wasm-pack test --node`. What this native test pins is the property that
//! keeps those assertions true for call sites they do not name: no source
//! file reaches `serde_wasm_bindgen`'s serializer except through
//! `js_value.rs`. A new `to_value` call would bring back the ES2015 `Map`
//! records the issue was about, for whichever type first carries a map.

use std::path::{Path, PathBuf};

/// Files allowed to name the serializer: the helper and this guard.
const ALLOWED: [&str; 2] = ["js_value.rs", "js_value_tests.rs"];

/// Spellings that reach `serde_wasm_bindgen`'s serializer directly.
const FORBIDDEN: [&str; 2] = [
    "serde_wasm_bindgen::to_value",
    "serde_wasm_bindgen::Serializer",
];

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("src/ is readable") {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn only_js_value_rs_reaches_the_serde_wasm_bindgen_serializer() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_sources(&src, &mut files);
    assert!(
        files.iter().any(|f| f.ends_with("store_search.rs")),
        "the scan must see the crate's sources, found {} files under {}",
        files.len(),
        src.display()
    );

    let offenders: Vec<String> = files
        .iter()
        .filter(|f| !ALLOWED.iter().any(|name| f.ends_with(name)))
        .flat_map(|f| {
            let text = std::fs::read_to_string(f).expect("source is UTF-8");
            text.lines()
                .enumerate()
                .filter(|(_, line)| FORBIDDEN.iter().any(|needle| line.contains(needle)))
                .map(|(n, line)| format!("{}:{}: {}", f.display(), n + 1, line.trim()))
                .collect::<Vec<_>>()
        })
        .collect();

    assert!(
        offenders.is_empty(),
        "serialize through crate::js_value::to_js_value instead:\n{}",
        offenders.join("\n")
    );
}
