use super::{Capability, CollectionInventory, SourceProvenance};
use crate::migration::diagnostic_copy::DiagnosticCopy;
use crate::migration::enumeration::AGENT_COLLECTIONS;
use std::collections::BTreeMap;

/// The complete v4 capability vocabulary, in `BTreeMap` iteration order.
pub(super) const DIAGNOSIS_CAPABILITY_KEYS: [&str; 9] = [
    "diagnostic_staging",
    "disk_headroom",
    "edge_counts",
    "edge_export",
    "embedder_cost",
    "inventory",
    "source_access_is_read_only",
    "source_provenance",
    "switch_same_filesystem",
];

/// Destination capacity is distinct from the staging proof done by diagnosis.
pub(super) const NO_HEADROOM: &str =
    "free space for the future rebuilt destination is not established. Diagnostic staging is \
     checked separately and does not prove that a different destination volume can hold the \
     rebuilt vectors. Supply and measure the final destination before reconstruction.";

/// The embedder's cost per fact is the one number a `reembed` rebuild's
/// duration turns on, and the one this gate still cannot produce.
///
/// This text used to quote `16.3 us/fact` as the thing to weigh the embedder
/// against. That number is the store's re-insertion cost measured at DIM=4 on
/// payloads carrying no text, and presenting it beside an embedder cost made a
/// regime-dependent claim look settled — which is what opened #1815.
pub(super) const NO_EMBEDDER_COST: &str =
    "the embedding cost per fact is NOT established for the target model. What #1816 measured, on \
     one machine in a debug build with one embedder call per fact, is a RATIO: re-embedding cost \
     about 23x a re-insertion (88 462 us against 3 900 us per fact, bge-m3 at 1024 dimensions). \
     That figure licenses no duration promise. It is one model, unbatched, unoptimised, and the \
     dominant term in its denominator turned out to be the payload's BM25 text indexing rather \
     than the vector's width — so it does not transfer to another embedder, another backend, or a \
     batched run. Under `reuse` the embedder is never called and this cost is zero. Before any \
     duration is quoted to an operator, it has to be measured against the actual target model on \
     the actual machine.";

/// An unrecorded source model makes an equal-width swap invisible.
const NO_PROVENANCE: &str =
    "the source model is not recorded, so a model change at EQUAL width cannot be detected — the \
     vectors would be silently incomparable. The operator has to state the source model; it \
     cannot be discovered.";

/// Counting relations is not an export contract.
pub(super) const NO_EDGE_EXPORT: &str =
    "the public diagnosis/rebuild path can count outgoing relations, but it does not export a \
     complete stream of edge tuples (stable edge id, source, target, label and properties). \
     Reconstructing edges from an external list would only prove that list agrees with itself, \
     not that every source edge was preserved. A lossless edge export and reinsertion API must \
     exist and be tested before reconstruction.";

/// A canonical missing verdict used both when generating and validating v4.
pub(super) fn missing_capability(blocker: &str) -> Capability {
    Capability::Missing {
        blocker: blocker.to_owned(),
    }
}

/// Require a derived capability to match its canonical v4 verdict.
pub(super) fn require_canonical_capability(
    capabilities: &BTreeMap<String, Capability>,
    name: &str,
    expected: &Capability,
) -> Result<(), String> {
    let actual = capabilities
        .get(name)
        .ok_or_else(|| format!("diagnosis capability `{name}` is missing"))?;
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "diagnosis capability `{name}` is inconsistent with its report fields: expected {expected:?}, got {actual:?}"
        ))
    }
}

/// Whether a rename-based switch can be atomic on the observed topology.
pub(in crate::migration) fn switch_filesystem_capability(
    same_filesystem: Option<bool>,
) -> Capability {
    match same_filesystem {
        Some(true) => Capability::Proven {
            evidence: "source and destination resolve to the same filesystem, so the prepared \
                       directory can be switched into place with one filesystem rename"
                .to_owned(),
        },
        Some(false) => Capability::Missing {
            blocker: "source and destination are on different filesystems. The switch contract \
                      is a filesystem rename, which cannot atomically cross that boundary; choose \
                      a destination on the source filesystem"
                .to_owned(),
        },
        None => Capability::Missing {
            blocker: "source and destination have not been proven to share a filesystem. The \
                      switch contract is a filesystem rename, so unknown topology cannot be \
                      treated as atomically switchable; supply a destination and establish the \
                      comparison"
                .to_owned(),
        },
    }
}

/// Reconcile the recorded source embedder with the store and target contracts.
pub(super) fn provenance_capability(
    provenance: &SourceProvenance,
    source_dimension: Option<usize>,
    target_model: &str,
    target_dimension: usize,
) -> Capability {
    let SourceProvenance::Known { model, dimension } = provenance else {
        return missing_capability(NO_PROVENANCE);
    };
    let Some(collection_dimension) = source_dimension else {
        return Capability::Missing {
            blocker: format!(
                "the source provenance records model '{model}' at {dimension} dimensions, but \
                 the source collections do not establish one shared dimension. The record cannot \
                 be reconciled with the data it claims to describe"
            ),
        };
    };
    if *dimension != collection_dimension {
        return Capability::Missing {
            blocker: format!(
                "the source provenance records model '{model}' at {dimension} dimensions, but \
                 the source collections are {collection_dimension}-dimensional. At least one side \
                 does not describe the vectors on disk"
            ),
        };
    }
    if model == target_model && *dimension != target_dimension {
        return Capability::Missing {
            blocker: format!(
                "model '{model}' is recorded at {dimension} dimensions in the source but declared \
                 at {target_dimension} dimensions for the target. One model identity cannot \
                 satisfy both contracts; use the correct target model identifier or dimension"
            ),
        };
    }
    Capability::Proven {
        evidence: format!(
            "the source records model '{model}' at {dimension} dimensions, matching the shared \
             collection width; the target contract is '{target_model}' at {target_dimension} \
             dimensions"
        ),
    }
}

/// Turn missing capabilities and absent collections into operator-facing gates.
pub(super) fn blockers_for(
    capabilities: &BTreeMap<String, Capability>,
    collections: &[CollectionInventory],
) -> Vec<String> {
    capabilities
        .iter()
        .filter_map(|(name, cap)| match cap {
            Capability::Missing { blocker } => Some(format!("{name}: {blocker}")),
            Capability::Proven { .. } => None,
        })
        .chain(
            collections
                .iter()
                .filter(|c| !c.present)
                .map(|c| format!("collection `{}` is absent from the store", c.name)),
        )
        .collect()
}

/// Every capability the rebuild depends on, with its verdict.
pub(super) fn capability_map(
    provenance: &SourceProvenance,
    source_dimension: Option<usize>,
    target_model: &str,
    target_dimension: usize,
    edge_counts: Capability,
    same_filesystem: Option<bool>,
    copy: &DiagnosticCopy,
) -> BTreeMap<String, Capability> {
    BTreeMap::from([
        ("diagnostic_staging".to_owned(), staging_capability(copy)),
        ("disk_headroom".to_owned(), missing_capability(NO_HEADROOM)),
        ("edge_counts".to_owned(), edge_counts),
        ("edge_export".to_owned(), missing_capability(NO_EDGE_EXPORT)),
        (
            "embedder_cost".to_owned(),
            missing_capability(NO_EMBEDDER_COST),
        ),
        ("inventory".to_owned(), inventory_capability()),
        (
            "source_access_is_read_only".to_owned(),
            read_only_capability(copy),
        ),
        (
            "source_provenance".to_owned(),
            provenance_capability(provenance, source_dimension, target_model, target_dimension),
        ),
        (
            "switch_same_filesystem".to_owned(),
            switch_filesystem_capability(same_filesystem),
        ),
    ])
}

fn inventory_capability() -> Capability {
    Capability::Proven {
        evidence: format!(
            "all {} agent collections were looked up by name and walked by cursor; absent \
             ones are reported as absent rather than skipped.",
            AGENT_COLLECTIONS.len()
        ),
    }
}

fn read_only_capability(copy: &DiagnosticCopy) -> Capability {
    Capability::Proven {
        evidence: format!(
            "the live source was never passed to Database::open; it matched content fingerprint '{}' before capture, after capture, and after inventory of the verified copy",
            copy.source_fingerprint()
        ),
    }
}

fn staging_capability(copy: &DiagnosticCopy) -> Capability {
    Capability::Proven {
        evidence: format!(
            "{} bytes were available on the staging volume before copying; {} bytes were required for the {}-byte source plus fixed and percentage headroom",
            copy.staging_available(),
            copy.staging_required(),
            copy.source_bytes()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_canonical_capability_is_a_validation_error() {
        let result = require_canonical_capability(
            &BTreeMap::new(),
            "disk_headroom",
            &missing_capability(NO_HEADROOM),
        );

        assert_eq!(
            result,
            Err("diagnosis capability `disk_headroom` is missing".to_owned())
        );
    }
}
