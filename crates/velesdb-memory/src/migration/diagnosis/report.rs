use super::capabilities::{
    blockers_for, edge_export_capability, missing_capability, provenance_capability,
    require_canonical_capability, DIAGNOSIS_CAPABILITY_KEYS, NO_EMBEDDER_COST, NO_HEADROOM,
};
use super::{
    assess, resolve, switch_filesystem_capability, Capability, CollectionInventory,
    DiagnosisReport, Resolution, SourceProvenance, Strategy, DIAGNOSIS_FORMAT_VERSION,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// Wire shape read before the v4 invariants are trusted.
///
/// Keeping this private prevents callers from accidentally treating successful
/// JSON decoding as a validated diagnosis. The public type is only produced
/// after [`DiagnosisReport::validate`] accepts every derived field.
#[derive(serde::Deserialize)]
struct UncheckedDiagnosisReport {
    format_version: u32,
    source_path: PathBuf,
    source_fingerprint: String,
    source_dimension: Option<usize>,
    source_provenance: SourceProvenance,
    target_model: String,
    target_dimension: usize,
    requested_strategy: Strategy,
    resolution: Resolution,
    collections: Vec<CollectionInventory>,
    facts: u64,
    edges: u64,
    working_contexts: u64,
    reserved_metadata: BTreeSet<String>,
    ttl_summary: super::TtlSummary,
    bytes_on_disk: u64,
    diagnostic_staging_required: u64,
    diagnostic_staging_available: u64,
    disk_headroom: Option<u64>,
    same_filesystem: Option<bool>,
    capabilities: BTreeMap<String, Capability>,
    blockers: Vec<String>,
}

impl TryFrom<UncheckedDiagnosisReport> for DiagnosisReport {
    type Error = String;

    fn try_from(unchecked: UncheckedDiagnosisReport) -> Result<Self, Self::Error> {
        let UncheckedDiagnosisReport {
            format_version,
            source_path,
            source_fingerprint,
            source_dimension,
            source_provenance,
            target_model,
            target_dimension,
            requested_strategy,
            resolution,
            collections,
            facts,
            edges,
            working_contexts,
            reserved_metadata,
            ttl_summary,
            bytes_on_disk,
            diagnostic_staging_required,
            diagnostic_staging_available,
            disk_headroom,
            same_filesystem,
            capabilities,
            blockers,
        } = unchecked;
        let report = Self {
            format_version,
            source_path,
            source_fingerprint,
            source_dimension,
            source_provenance,
            target_model,
            target_dimension,
            requested_strategy,
            resolution,
            collections,
            facts,
            edges,
            working_contexts,
            reserved_metadata,
            ttl_summary,
            bytes_on_disk,
            diagnostic_staging_required,
            diagnostic_staging_available,
            disk_headroom,
            same_filesystem,
            capabilities,
            blockers,
        };
        report.validate()?;
        Ok(report)
    }
}

impl<'de> serde::Deserialize<'de> for DiagnosisReport {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let unchecked =
            <UncheckedDiagnosisReport as serde::Deserialize>::deserialize(deserializer)?;
        Self::try_from(unchecked).map_err(serde::de::Error::custom)
    }
}

impl DiagnosisReport {
    /// Parse a persisted report, refusing a shape from another version before
    /// deserializing the rest of it.
    ///
    /// Reading `format_version` first is deliberate: a future report may add
    /// fields or change representations this binary cannot understand. Such a
    /// report is an incompatible artifact, not corrupt JSON and not something
    /// to interpret approximately.
    ///
    /// # Errors
    /// Returns a descriptive refusal when the JSON is malformed, has no
    /// integer `format_version`, or was produced with a different format.
    pub fn parse(json: &str) -> Result<Self, String> {
        let value: Value = serde_json::from_str(json)
            .map_err(|err| format!("cannot parse diagnosis report JSON: {err}"))?;
        let version = value
            .get("format_version")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                "diagnosis report has no unsigned integer `format_version`; refusing an unversioned artifact"
                    .to_owned()
            })?;
        validate_format_version(version)?;
        serde_json::from_value(value)
            .map_err(|err| format!("cannot parse diagnosis report version {version}: {err}"))
    }

    /// Validate every v5 field that is derived rather than independently
    /// observed.
    fn validate(&self) -> Result<(), String> {
        validate_format_version(u64::from(self.format_version))?;
        validate_capability_keys(&self.capabilities)?;

        for (name, expected) in canonical_capabilities(self) {
            require_canonical_capability(&self.capabilities, name, &expected)?;
        }
        validate_uncalculated_capability_shapes(&self.capabilities)?;

        validate_resolution(self)?;
        validate_blockers(self)
    }

    /// Whether a rebuild could proceed with no outstanding question.
    ///
    /// Expect `false` until every environmental question (source provenance,
    /// destination capacity and real embedder cost included) has evidence.
    #[must_use]
    pub fn is_clear(&self) -> bool {
        self.validate().is_ok()
            && !self.capabilities.is_empty()
            && self.blockers.is_empty()
            && self.capabilities.values().all(Capability::is_proven)
    }
}

fn validate_capability_keys(capabilities: &BTreeMap<String, Capability>) -> Result<(), String> {
    let actual_keys: Vec<&str> = capabilities.keys().map(String::as_str).collect();
    if actual_keys == DIAGNOSIS_CAPABILITY_KEYS {
        return Ok(());
    }
    Err(format!(
        "diagnosis report capabilities are incomplete or unknown: expected exactly {DIAGNOSIS_CAPABILITY_KEYS:?}, got {actual_keys:?}"
    ))
}

/// Hold the four capabilities the parser cannot recalculate to their SHAPE.
///
/// Their evidence embeds measurements only the live diagnosis could take —
/// staging room, edge counts, the read-only fingerprint check — so a parsed
/// report cannot re-derive the text. Two shape facts survive parsing anyway:
/// three of them have no Missing-producing code path in `diagnose`, so a
/// Missing one was not produced by a diagnosis; and evidence or blocker text
/// that is empty says nothing, which no capability of any provenance does.
fn validate_uncalculated_capability_shapes(
    capabilities: &BTreeMap<String, Capability>,
) -> Result<(), String> {
    for name in [
        "diagnostic_staging",
        "inventory",
        "source_access_is_read_only",
    ] {
        if let Some(Capability::Missing { .. }) = capabilities.get(name) {
            return Err(format!(
                "diagnosis capability `{name}` is inconsistent with its report \
                 fields: no diagnosis produces it as Missing"
            ));
        }
    }
    for (name, capability) in capabilities {
        let text = match capability {
            Capability::Proven { evidence } => evidence,
            Capability::Missing { blocker } => blocker,
        };
        if text.trim().is_empty() {
            return Err(format!(
                "diagnosis capability `{name}` carries an empty text; evidence \
                 that says nothing is not evidence"
            ));
        }
    }
    Ok(())
}

fn canonical_capabilities(report: &DiagnosisReport) -> [(&'static str, Capability); 5] {
    [
        ("edge_export", edge_export_capability()),
        ("disk_headroom", missing_capability(NO_HEADROOM)),
        ("embedder_cost", missing_capability(NO_EMBEDDER_COST)),
        (
            "switch_same_filesystem",
            switch_filesystem_capability(report.same_filesystem),
        ),
        (
            "source_provenance",
            provenance_capability(
                &report.source_provenance,
                report.source_dimension,
                &report.target_model,
                report.target_dimension,
            ),
        ),
    ]
}

/// Refuse a report whose stated regime does not follow from its own fields.
///
/// The regime is the one thing in a diagnosis an operator acts on, and it is
/// derived — so a report carrying a hand-edited `resolution`, or one produced by
/// a build whose rule differed, must not be read as authority. Recomputing it
/// here is what makes "this report says REUSE" mean "this store permits reuse"
/// rather than "this file contains the word REUSE".
fn validate_resolution(report: &DiagnosisReport) -> Result<(), String> {
    let expected = resolve(
        report.requested_strategy,
        assess(
            &report.source_provenance,
            report.source_dimension,
            &report.target_model,
            report.target_dimension,
        ),
    );
    if report.resolution == expected {
        return Ok(());
    }
    Err(format!(
        "diagnosis report states resolution {:?} for a {:?} request, but its own provenance and \
         target contract resolve to {expected:?}",
        report.resolution, report.requested_strategy
    ))
}

fn validate_blockers(report: &DiagnosisReport) -> Result<(), String> {
    let expected = blockers_for(&report.capabilities, &report.collections);
    if report.blockers == expected {
        return Ok(());
    }
    Err(format!(
        "diagnosis report blockers do not match its capabilities and absent collections: expected {expected:?}, got {:?}",
        report.blockers
    ))
}

/// Refuse a report from a binary with different diagnosis semantics.
fn validate_format_version(version: u64) -> Result<(), String> {
    if version == u64::from(DIAGNOSIS_FORMAT_VERSION) {
        return Ok(());
    }
    let relation = if version < u64::from(DIAGNOSIS_FORMAT_VERSION) {
        "older"
    } else {
        "newer"
    };
    Err(format!(
        "diagnosis report format version {version} is {relation} than the supported version {DIAGNOSIS_FORMAT_VERSION}; run a fresh diagnosis with this binary"
    ))
}
