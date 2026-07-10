//! Stable-hash conformance vectors (Requirement 7.1).

use super::Divergence;

/// One stable-hash reference case: a UTF-8 input and its frozen FNV-1a
/// (`hash_id`) output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HashVector {
    /// The input string fed to the stable hasher.
    pub input: &'static str,
    /// The frozen golden `u64` the stable hasher must produce.
    pub expected: u64,
}

/// The frozen golden table for the stable string→`u64` derivation.
///
/// Values are the FNV-1a output of [`crate::hash_id`] and are part of the
/// cross-engine contract: they MUST NOT change without a coordinated,
/// versioned migration, because persisted/inter-node IDs depend on them.
#[must_use]
pub fn hash_reference_vectors() -> Vec<HashVector> {
    vec![
        // The empty string hashes to the FNV-1a offset basis.
        HashVector {
            input: "",
            expected: 0xcbf2_9ce4_8422_2325,
        },
        HashVector {
            input: "veles",
            expected: 0x6ccb_dfb6_eb43_fc1c,
        },
        HashVector {
            input: "tenant:acme",
            expected: 0x434a_088f_8b77_5207,
        },
        HashVector {
            input: "point:42",
            expected: 0x8db6_5f18_995c_fdd7,
        },
        HashVector {
            input: "hello world",
            expected: 0x779a_65e7_023c_d2e7,
        },
        // Multi-byte UTF-8 (é is two bytes) exercises byte-level folding.
        HashVector {
            input: "café",
            expected: 0x48e8_823a_cfa4_0d89,
        },
    ]
}

/// Runs every stable-hash reference case against `hash_fn` and returns the
/// diverging cases (empty = full agreement).
///
/// Pass [`crate::hash_id`] to verify core, or any candidate `Fn(&str) -> u64`
/// (e.g. premium's own derivation) to verify agreement with core
/// (Requirement 7.5).
#[must_use]
pub fn check_stable_hash(hash_fn: impl Fn(&str) -> u64) -> Vec<Divergence> {
    let mut divergences = Vec::new();
    for vector in hash_reference_vectors() {
        let actual = hash_fn(vector.input);
        if actual != vector.expected {
            divergences.push(Divergence {
                case: format!("hash_id({:?})", vector.input),
                expected: format!("{:#018x}", vector.expected),
                actual: format!("{actual:#018x}"),
            });
        }
    }
    divergences
}
