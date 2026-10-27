//! Refusing a request before any stage runs: budget, policy and media fragments.

use super::media;
use super::{CompilePolicy, CompileRequest, ContextFragment};
use crate::error::MemoryError;
use crate::limits;

/// Reject requests over the [`crate::limits`] caps and compute the usable
/// budget (`clamped budget − reserve`).
pub(super) fn validate(
    request: &CompileRequest,
    policy: &CompilePolicy,
) -> Result<u64, MemoryError> {
    // The pure core never resolves `path` (V2b-1): that is an adapter-side
    // I/O pre-pass (`context::ingest::resolve_fragments`) that clears the
    // field on success. A `path` still set here means either no adapter ran
    // (e.g. a binding with no ingest support, such as the WASM build) or
    // ingestion is disabled — both report the same explicit error rather
    // than silently compiling an empty-content fragment.
    if request.fragments.iter().any(|f| f.path.is_some()) {
        return Err(MemoryError::IngestDisabled);
    }
    if request.fragments.len() > limits::MAX_FRAGMENTS {
        return Err(MemoryError::ContextOverLimit(format!(
            "{} fragments exceed the cap of {}",
            request.fragments.len(),
            limits::MAX_FRAGMENTS
        )));
    }
    if let Some(oversized) = request
        .fragments
        .iter()
        .find(|fragment| fragment.content.len() > limits::MAX_FRAGMENT_BYTES)
    {
        return Err(MemoryError::ContextOverLimit(format!(
            "a fragment of {} bytes exceeds the cap of {} bytes",
            oversized.content.len(),
            limits::MAX_FRAGMENT_BYTES
        )));
    }
    for fragment in &request.fragments {
        let Some(metadata) = fragment.metadata.as_ref() else {
            continue;
        };
        let bytes = limits::metadata_bytes(metadata);
        if bytes > limits::MAX_METADATA_BYTES {
            return Err(MemoryError::MetadataTooLarge {
                bytes,
                max: limits::MAX_METADATA_BYTES,
            });
        }
    }
    validate_media(&request.fragments)?;
    let budget = limits::clamp_token_budget(request.token_budget);
    let usable = budget.saturating_sub(policy.response_reserve_tokens);
    if usable == 0 {
        return Err(MemoryError::ContextBudget {
            budget,
            reserve: policy.response_reserve_tokens,
        });
    }
    Ok(usable)
}

/// Reject a fragment whose media payload violates
/// [`limits::MAX_MEDIA_BYTES`] or is not well-formed base64 — checked eagerly
/// here, before any decoding/hashing/estimation downstream, so a malformed
/// payload never reaches the pipeline (fail fast, same `INVALID_PARAMS`
/// shape as every other cap in [`validate`]).
pub(super) fn validate_media(fragments: &[ContextFragment]) -> Result<(), MemoryError> {
    let mut total_media_bytes: usize = 0;
    for (seq, fragment) in fragments.iter().enumerate() {
        let Some(media_ref) = &fragment.media else {
            continue;
        };
        total_media_bytes = total_media_bytes.saturating_add(media_ref.bytes_b64.len());
        if total_media_bytes > limits::MAX_TOTAL_MEDIA_BYTES {
            return Err(MemoryError::ContextOverLimit(format!(
                "total media payload exceeds the request cap of {} base64 bytes",
                limits::MAX_TOTAL_MEDIA_BYTES
            )));
        }
        if media_ref.bytes_b64.len() > limits::MAX_MEDIA_BYTES {
            return Err(MemoryError::ContextOverLimit(format!(
                "fragment #{seq} media payload of {} base64 bytes exceeds the cap of {} bytes",
                media_ref.bytes_b64.len(),
                limits::MAX_MEDIA_BYTES
            )));
        }
        if !media::is_valid_base64(&media_ref.bytes_b64) {
            return Err(MemoryError::ContextOverLimit(format!(
                "fragment #{seq} media payload is not valid base64"
            )));
        }
    }
    Ok(())
}
