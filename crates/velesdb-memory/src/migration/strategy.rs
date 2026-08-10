//! Which regime a rebuild runs in, and why (#1815).
//!
//! A rebuild has two possible regimes and the module below the CLI is
//! deliberately neutral between them, because the caller supplies the vector:
//! reuse the source vectors, or re-embed every fact with the target model. The
//! arbitration on #1815 is that **reuse is permitted only where compatibility
//! is proven, and every other case re-embeds**.
//!
//! # The one thing this file exists to refuse
//!
//! Two models that happen to produce the same width do NOT produce comparable
//! vectors, and nothing on disk distinguishes them: the store records a
//! dimension, and — only since #1751, and only for a store that was empty when
//! it was first opened — a model name. So an equal-width model swap is
//! **invisible**, and a rebuild that inferred the source model from the width
//! would reuse vectors from one model in a store that claims another. Recall
//! would then return nonsense without a single error anywhere.
//!
//! Hence: an unrecorded source model is never guessed at. It resolves to
//! re-embedding, which is always sound because it reads the stored *text* and
//! never the stored vector.
//!
//! # There is deliberately no `force-reuse`
//!
//! An override that reused vectors against an unproven provenance would be an
//! official route to a semantically incoherent store, so [`Strategy::parse`]
//! names it and refuses it rather than leaving an operator to discover it does
//! not exist. `--strategy reembed` is the escape hatch, and it escapes towards
//! the *safe* regime.

use super::SourceProvenance;

// ---------------------------------------------------------------------------
// THE VOCABULARY
//
// A closed set of five sentences, so that every diagnostic an operator can see
// is one of five and each is tested. A `format!` at the call site would let a
// sixth phrasing appear without anybody deciding it should.
// ---------------------------------------------------------------------------

/// The source records the target model at the target width.
const MATCH: &str = "source and target embedding provenance match";
/// A recorded model that is not the target's — including at equal width.
const MODEL_DIFFERS: &str = "target model differs";
/// The recorded width is not the target's.
const DIMENSION_DIFFERS: &str = "target dimension differs";
/// No record at all: the nominal case for every store predating #1751.
const PROVENANCE_UNKNOWN: &str = "source provenance is unknown";
/// A record that disagrees with the vectors it claims to describe.
const PROVENANCE_CONTRADICTS: &str = "source provenance contradicts the stored dimension";

// ---------------------------------------------------------------------------
// WHAT THE STORE PERMITS
// ---------------------------------------------------------------------------

/// What the source's own record permits, independently of what was asked for.
///
/// Split from [`Strategy`] on purpose: this is a property of the store, and
/// mixing it with the operator's request is what would make "the operator asked
/// nicely" look like a reason vectors are compatible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Compatibility {
    /// Known provenance, same model, same width. The only value that permits
    /// reuse.
    Match,
    /// Known provenance naming a different model — **including at equal
    /// width**, which is exactly the case no measurement can detect.
    ModelDiffers,
    /// Known provenance at a width the target does not produce.
    DimensionDiffers,
    /// The store records nothing. Not a fault: the nominal state of every store
    /// created before the record existed, including the one #1762 was opened
    /// for.
    ProvenanceUnknown,
    /// The record and the collections disagree, or the collections establish no
    /// shared width at all. Neither side can be trusted to describe the other.
    ProvenanceContradictsDimension,
}

impl Compatibility {
    /// The sentence that names this state, from the closed vocabulary.
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            Self::Match => MATCH,
            Self::ModelDiffers => MODEL_DIFFERS,
            Self::DimensionDiffers => DIMENSION_DIFFERS,
            Self::ProvenanceUnknown => PROVENANCE_UNKNOWN,
            Self::ProvenanceContradictsDimension => PROVENANCE_CONTRADICTS,
        }
    }

    /// Whether reusing the source vectors is defensible.
    ///
    /// One variant, and it is the point of the whole file: proof, not absence
    /// of evidence.
    #[must_use]
    pub fn permits_reuse(self) -> bool {
        matches!(self, Self::Match)
    }

    /// Every variant, so an exhaustive check cannot silently miss one added
    /// later.
    #[must_use]
    pub fn all() -> Vec<Self> {
        vec![
            Self::Match,
            Self::ModelDiffers,
            Self::DimensionDiffers,
            Self::ProvenanceUnknown,
            Self::ProvenanceContradictsDimension,
        ]
    }
}

/// Read the source's record against the target contract.
///
/// The record is checked against the DATA first: a provenance naming a width
/// the collections do not have describes something other than this store, and
/// reading its model name off it would be reading a record already shown wrong.
/// `source_dimension` is `None` when the collections disagree or the store has
/// none, which fails the same reconciliation for the same reason.
#[must_use]
pub fn assess(
    provenance: &SourceProvenance,
    source_dimension: Option<usize>,
    target_model: &str,
    target_dimension: usize,
) -> Compatibility {
    let SourceProvenance::Known { model, dimension } = provenance else {
        return Compatibility::ProvenanceUnknown;
    };
    if source_dimension != Some(*dimension) {
        return Compatibility::ProvenanceContradictsDimension;
    }
    // Model before width: a model change is the fact that matters, and at equal
    // width it is the ONLY thing that distinguishes two incomparable stores.
    if model != target_model {
        return Compatibility::ModelDiffers;
    }
    if *dimension != target_dimension {
        return Compatibility::DimensionDiffers;
    }
    Compatibility::Match
}

// ---------------------------------------------------------------------------
// WHAT THE OPERATOR ASKED FOR
// ---------------------------------------------------------------------------

/// What the operator selected on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    /// Decide from the source's own record. The default.
    Auto,
    /// Reuse the source vectors — honoured only against a proven match.
    Reuse,
    /// Re-embed every fact. Always available.
    Reembed,
}

impl Strategy {
    /// Parse a `--strategy` value.
    ///
    /// # Errors
    /// Names the three accepted values. `force-reuse`, in any spelling, gets
    /// its own message: it is refused by design rather than merely absent, and
    /// an operator who reached for it is asking the one question this batch
    /// answered with "no".
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "auto" => Ok(Self::Auto),
            "reuse" => Ok(Self::Reuse),
            "reembed" => Ok(Self::Reembed),
            "force-reuse" | "force_reuse" => Err(format!(
                "--strategy {value} does not exist, and not by oversight: reusing vectors against \
                 an unproven provenance is an official route to a store whose vectors and whose \
                 recorded model disagree, which recall would answer from without ever failing. \
                 Use --strategy reembed to rebuild from the stored text"
            )),
            other => Err(format!(
                "--strategy expects auto, reuse or reembed, got {other:?}"
            )),
        }
    }

    /// Every variant, so an exhaustive check cannot silently miss one added
    /// later.
    #[must_use]
    pub fn all() -> Vec<Self> {
        vec![Self::Auto, Self::Reuse, Self::Reembed]
    }
}

// ---------------------------------------------------------------------------
// WHAT WILL HAPPEN
// ---------------------------------------------------------------------------

/// What the rebuild will do, or why it will not run.
///
/// There is no variant meaning "reuse, but flagged": a rebuild either reuses
/// vectors it has grounds to reuse, or it does not reuse them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "resolution", rename_all = "snake_case")]
pub enum Resolution {
    /// Carry the source vectors across unchanged. The target embedder is never
    /// called.
    Reuse,
    /// Compute every vector with the target embedder, from the stored text.
    Reembed {
        /// What made reuse unavailable — or, under `--strategy reembed`, what
        /// the store would have permitted anyway.
        because: Compatibility,
    },
    /// Change nothing.
    Refuse {
        /// The state of the store that makes the request unanswerable.
        because: Compatibility,
        /// What was asked for, which decides what the operator should do next.
        requested: Strategy,
    },
}

impl Resolution {
    /// The one line, from the closed vocabulary, that names this decision.
    #[must_use]
    pub fn diagnostic(self) -> String {
        match self {
            Self::Reuse => format!("REUSE: {MATCH}"),
            Self::Reembed { because } => format!("REEMBED: {}", because.reason()),
            Self::Refuse {
                because,
                requested: Strategy::Reuse,
            } => format!("REFUSE: reuse was requested, but {}", because.reason()),
            Self::Refuse { because, .. } => format!("REFUSE: {}", because.reason()),
        }
    }

    /// What the operator can do about it, or `None` when nothing is wrong.
    ///
    /// A refusal that named only the problem would leave an operator with a
    /// store they cannot migrate and no next step; both refusals have one, and
    /// neither is "reuse it anyway".
    #[must_use]
    pub fn guidance(self) -> Option<&'static str> {
        match self {
            Self::Reuse | Self::Reembed { .. } => None,
            Self::Refuse {
                requested: Strategy::Reuse,
                ..
            } => Some(
                "reuse is legitimate only when the source records the target model at the target \
                 width. Re-run with --strategy auto to let the source's own record decide, or \
                 --strategy reembed to rebuild every vector from the stored text.",
            ),
            Self::Refuse { .. } => Some(
                "the record and the vectors cannot both be right, so neither is read as truth. \
                 Re-run with --strategy reembed to rebuild from the stored text on the measured \
                 width — it never reads a source vector, so the contradiction cannot propagate.",
            ),
        }
    }

    /// Whether this decision runs a rebuild at all.
    #[must_use]
    pub fn runs(self) -> bool {
        !matches!(self, Self::Refuse { .. })
    }
}

/// Decide the regime from what was asked and what the store permits.
///
/// `Auto` refuses only the self-contradicting store: re-embedding is otherwise
/// always sound, since it reads the stored text and never the stored vector.
/// That refusal is not a dead end — `--strategy reembed` performs exactly the
/// rebuild `Auto` declined to choose on the operator's behalf.
#[must_use]
pub fn resolve(requested: Strategy, compatibility: Compatibility) -> Resolution {
    // Reuse, and only against a proven match. `reembed` is excluded even here:
    // an operator who named the safe regime gets it, whatever the store would
    // have permitted.
    if requested != Strategy::Reembed && compatibility.permits_reuse() {
        return Resolution::Reuse;
    }
    // Reuse asked for and not earned above, or `auto` meeting a store whose
    // record and vectors contradict each other — the one state `auto` will not
    // choose on the operator's behalf.
    let unearned_reuse = requested == Strategy::Reuse;
    let unreadable_store = requested == Strategy::Auto
        && compatibility == Compatibility::ProvenanceContradictsDimension;
    if unearned_reuse || unreadable_store {
        return Resolution::Refuse {
            because: compatibility,
            requested,
        };
    }
    // Everything else re-embeds: it reads the stored text, so no property of
    // the source's vectors can make it unsound.
    Resolution::Reembed {
        because: compatibility,
    }
}
