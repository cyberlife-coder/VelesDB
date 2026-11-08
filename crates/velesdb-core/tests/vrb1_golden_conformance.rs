//! VRB1 golden-bytes conformance (#2095 C4).
//!
//! `conformance/vrb1_golden.json` holds fixed `(ids, vectors, dim)` batches
//! and the exact bytes the wire encoder must emit. The TS SDK's
//! `encodeRawBulk` test reads the same file, so core's encoder and the SDK's
//! are pinned to the same octets — before this fixture each side pinned only
//! its own reading of the layout, and a drift in either direction compiled
//! green on both.
//!
//! The fixture is the contract; this file is only its runner. When a case is
//! added, the same hex must satisfy `encodeRawBulk` in `sdks/typescript`.

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    ids: Vec<u64>,
    vectors: Vec<f32>,
    dimension: usize,
    expect_hex: String,
}

fn fixture() -> Fixture {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance/vrb1_golden.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("fixture must be readable"))
        .expect("fixture must parse")
}

fn unhex(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("fixture hex must be valid"))
        .collect()
}

#[test]
fn vrb1_encode_matches_shared_golden_bytes() {
    let fixture = fixture();
    assert!(!fixture.cases.is_empty(), "fixture must hold cases");
    for case in &fixture.cases {
        assert_eq!(
            case.vectors.len(),
            case.ids.len() * case.dimension,
            "{}: flat vectors must be ids * dim",
            case.id
        );
        let got = velesdb_core::wire::vrb1::encode(&case.ids, &case.vectors, case.dimension);
        assert_eq!(
            got,
            unhex(&case.expect_hex),
            "{}: core encoder drifted from the shared golden bytes",
            case.id
        );
    }
}
