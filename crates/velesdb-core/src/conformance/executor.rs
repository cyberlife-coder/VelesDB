//! Shared-executor conformance vectors (Requirement 7.3).
//!
//! The reference operation is the canonical graph edge-id derivation
//! ([`crate::hash_edge_id`]): `(source, target, label) -> u64`. It is the
//! deterministic executor primitive that premium's forked graph/JOIN executor
//! depends on, and unlike a full query-executor reference it is persistence-
//! free and has a single, frozen input→output contract suitable for a golden
//! table.

use super::Divergence;

/// One executor reference case: an edge triple and its frozen edge-id output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutorVector {
    /// Source vertex id.
    pub source: u64,
    /// Target vertex id.
    pub target: u64,
    /// Edge label.
    pub label: &'static str,
    /// The frozen golden edge id the derivation must produce.
    pub expected: u64,
}

/// The frozen golden table for the shared edge-id executor operation.
///
/// Values are the output of [`crate::hash_edge_id`] (FNV-1a over the raw
/// little-endian endpoint bytes followed by the label bytes). Cases cover the
/// empty label, endpoint order-sensitivity, and the maximum endpoint value.
#[must_use]
pub fn executor_reference_vectors() -> Vec<ExecutorVector> {
    vec![
        ExecutorVector {
            source: 1,
            target: 2,
            label: "knows",
            expected: 0x083a_4358_f694_89c6,
        },
        // Order-sensitive: swapping endpoints yields a different id.
        ExecutorVector {
            source: 2,
            target: 1,
            label: "knows",
            expected: 0xa2e9_af24_0f80_a6a6,
        },
        ExecutorVector {
            source: 0,
            target: 0,
            label: "",
            expected: 0x8820_1fb9_60ff_6465,
        },
        ExecutorVector {
            source: 100,
            target: 200,
            label: "linked_to",
            expected: 0x9624_81e1_7255_9460,
        },
        ExecutorVector {
            source: u64::from(u32::MAX),
            target: 1,
            label: "e",
            expected: 0x07dc_c840_67e6_abff,
        },
    ]
}

/// Runs every shared-executor reference case against `edge_id_fn` and returns
/// the diverging cases (empty = full agreement).
///
/// Pass [`crate::hash_edge_id`] to verify core, or any candidate
/// `Fn(u64, u64, &str) -> u64` to verify a downstream executor's edge-id
/// derivation agrees with core (Requirement 7.5).
#[must_use]
pub fn check_executor(edge_id_fn: impl Fn(u64, u64, &str) -> u64) -> Vec<Divergence> {
    let mut divergences = Vec::new();
    for vector in executor_reference_vectors() {
        let actual = edge_id_fn(vector.source, vector.target, vector.label);
        if actual != vector.expected {
            divergences.push(Divergence {
                case: format!(
                    "hash_edge_id({}, {}, {:?})",
                    vector.source, vector.target, vector.label
                ),
                expected: format!("{:#018x}", vector.expected),
                actual: format!("{actual:#018x}"),
            });
        }
    }
    divergences
}
