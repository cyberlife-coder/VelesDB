//! What embedder filled this store, and whether the configured one can still
//! read it (#1751, arbitration A1).
//!
//! # The problem this exists for
//!
//! A store's vectors are only comparable to vectors from the **same embedding
//! model**. `velesdb-core` already refuses to open a collection whose
//! dimension differs from the embedder's, which catches the loud half of the
//! problem — `bge-m3` (1024) against `all-minilm` (384) fails immediately and
//! clearly.
//!
//! It does not catch the quiet half. Two different models of the *same*
//! dimension open perfectly and then return nonsense: this crate's own `hash`
//! embedder is 384-dimensional, and so is `all-minilm`. Recall degrades to
//! noise with nothing anywhere reporting a fault. Recording the model closes
//! that gap.
//!
//! # What is recorded, and what deliberately is not
//!
//! The model identifier and the dimension. **Not the backend.** A backend is a
//! *transport*: the same model served by Ollama, by oMLX or by a hosted
//! OpenAI-compatible API produces the same vectors, so refusing an open
//! because the transport changed would block a valid migration — precisely the
//! migration #1751 exists to enable.
//!
//! # When it is recorded
//!
//! Only when the store holds **no facts**. Never retroactively over data: a
//! single open with the wrong model would carve a false provenance into the
//! store, and every later check would trust it. A store that predates this
//! record therefore stays unrecorded for good, and its check degrades to the
//! dimension alone — with [`unrecorded_model_note`] saying so rather than
//! letting a successful open read as a verified match.

use std::path::Path;

/// Name of the record inside the store directory.
///
/// Sits beside the engine's own files rather than inside them: this is
/// `velesdb-memory`'s knowledge about its embedder, not something
/// `velesdb-core` — which only ever sees raw `&[f32]` — has any business
/// carrying.
pub const PROVENANCE_FILE: &str = "embedding-provenance.json";

/// The embedder a store was filled by.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EmbeddingProvenance {
    /// The model identifier as configured — `bge-m3`, `all-minilm`, or `hash`
    /// for this crate's built-in offline embedder.
    pub model: String,
    /// The vector width that model produces.
    pub dimension: usize,
}

impl EmbeddingProvenance {
    /// Record `model` at `dimension`.
    #[must_use]
    pub fn new(model: impl Into<String>, dimension: usize) -> Self {
        Self {
            model: model.into(),
            dimension,
        }
    }
}

/// Read the record from `store_dir`, or `None` when there is none.
///
/// An absent record is a normal outcome — every store created before this
/// existed has none — and is deliberately distinct from a failure to read one,
/// because the two lead to opposite actions: the first degrades the check, the
/// second stops the daemon.
///
/// # Errors
/// A record that exists but cannot be read or parsed. Treating that as
/// "absent" would silently disable the guard on exactly the store whose
/// metadata is already damaged; the message names the file so the operator can
/// delete it and let the check degrade knowingly.
pub fn read(store_dir: &Path) -> Result<Option<EmbeddingProvenance>, String> {
    let path = store_dir.join(PROVENANCE_FILE);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("cannot read {PROVENANCE_FILE}: {err}")),
    };
    // Unknown fields are IGNORED on purpose (no `deny_unknown_fields` here,
    // unlike the operator-facing config file): a store stamped by a newer
    // version must stay readable by an older binary, which can still run the
    // check it does understand. A typo is impossible — nobody writes this file
    // by hand.
    serde_json::from_str(&raw)
        .map(Some)
        .map_err(|err| format!("cannot parse {PROVENANCE_FILE}: {err} — delete it to reset the embedding record (the store's own data is untouched)"))
}

/// Write the record into `store_dir`.
///
/// # Errors
/// The directory is unwritable, or serialisation fails.
pub fn write(store_dir: &Path, provenance: &EmbeddingProvenance) -> Result<(), String> {
    let body = serde_json::to_string_pretty(provenance)
        .map_err(|err| format!("cannot serialise the embedding record: {err}"))?;
    std::fs::write(store_dir.join(PROVENANCE_FILE), body)
        .map_err(|err| format!("cannot write {PROVENANCE_FILE}: {err}"))
}

/// Compare what the store recorded against what the daemon is configured for.
///
/// `stored` is `None` for a store that predates the record; there is then
/// nothing to compare and the core's own dimension check remains the only
/// guard — see [`unrecorded_model_note`].
///
/// # Errors
/// A message naming **both** configurations and what can be done about them.
/// Naming only the mismatch would leave the operator guessing which side to
/// change.
pub fn check(
    stored: Option<&EmbeddingProvenance>,
    model: &str,
    dimension: usize,
) -> Result<(), String> {
    let Some(stored) = stored else {
        return Ok(());
    };
    if stored.model == model && stored.dimension == dimension {
        return Ok(());
    }
    // The dimension is stated on both sides even when only the name differs:
    // it is what tells an operator whether a re-index is a re-embed or a
    // rebuild, and a served model can change what its own name means.
    Err(format!(
        "this store was filled with the embedding model '{}' ({} dimensions), and the daemon is \
         configured for '{}' ({} dimensions). Vectors from two different models are not \
         comparable, so recall would silently return nonsense. Either point \
         VELESDB_MEMORY_EMBEDDER_MODEL back at '{}', or re-index the store against the new model \
         (re-indexing is not implemented yet — see #1751). Which backend serves the model does \
         not matter and is not recorded: the same model over Ollama, oMLX or an \
         OpenAI-compatible API produces the same vectors.",
        stored.model, stored.dimension, model, dimension, stored.model
    ))
}

/// What to say when the store carries no model record.
///
/// The disclosure is the point. A store created before this record existed
/// opens on a dimension match alone, and an operator reading that as "my model
/// matches" would be reading something nobody verified.
#[must_use]
pub fn unrecorded_model_note(model: &str) -> String {
    format!(
        "[velesdb-memory] this store predates embedding-model recording, so only the vector \
         dimension could be compared against '{model}' — not the model itself. Two different \
         models of the same width would pass this check. The record is written only for a store \
         with no facts in it, never over existing data, because a wrong stamp would be trusted \
         forever."
    )
}

#[cfg(test)]
#[path = "embedding_provenance_tests.rs"]
mod tests;
