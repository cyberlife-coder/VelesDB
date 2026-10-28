//! Persisted types and invariants for durable extraction jobs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::extract::Extraction;
use crate::model::RememberedExtraction;
use crate::service::Metadata;

pub(super) const RECORD_VERSION: u8 = 1;

/// Persisted lifecycle exposed by both extraction tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum ExtractionJobState {
    Accepted,
    Running,
    Committed,
    Failed,
}

impl ExtractionJobState {
    pub(super) fn is_terminal(self) -> bool {
        matches!(self, Self::Committed | Self::Failed)
    }
}

/// Lightweight receipt returned before model generation begins.
pub(super) struct JobReceipt {
    pub(super) request_id: String,
    pub(super) state: ExtractionJobState,
    pub(super) reused: bool,
}

/// Query view of one durable job.
pub(super) struct JobView {
    pub(super) request_id: String,
    pub(super) state: ExtractionJobState,
    pub(super) outcome: Option<JobOutcome>,
    pub(super) error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct JobOutcome {
    pub(super) ids: Vec<u64>,
    pub(super) skipped_over_cap: usize,
}

impl From<RememberedExtraction> for JobOutcome {
    fn from(value: RememberedExtraction) -> Self {
        Self {
            ids: value.ids,
            skipped_over_cap: value.skipped_over_cap,
        }
    }
}

/// Submission, status and durable-storage failures of the extraction jobs.
/// The MCP layer maps them to JSON-RPC codes; startup surfaces them from
/// [`McpServer::with_extraction_jobs`](super::McpServer::with_extraction_jobs).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ExtractionJobError {
    /// The request itself is unacceptable.
    #[error("{0}")]
    Invalid(String),
    /// The idempotency key was already used by a different request.
    #[error("idempotency key already belongs to a different extraction request")]
    Conflict,
    /// Too many jobs are pending.
    #[error("too many extraction jobs are pending; retry after one reaches a terminal state")]
    AtCapacity,
    /// No extraction backend is configured for the job.
    #[error(
        "extraction backend not configured: set VELESDB_MEMORY_EXTRACTOR, or pass \
         extractor='outline' for the offline deterministic reader"
    )]
    BackendNotConfigured,
    /// No job carries this request id.
    #[error("extraction request '{0}' does not exist")]
    NotFound(String),
    /// The durable job directory failed to open, read or write.
    #[error("durable extraction job storage failed: {0}")]
    Storage(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PersistedRequest {
    pub(super) text: String,
    pub(super) metadata: Option<Metadata>,
    pub(super) backend: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct JobRecord {
    pub(super) version: u8,
    pub(super) request_id: String,
    pub(super) input_digest: String,
    pub(super) state: ExtractionJobState,
    pub(super) request: Option<PersistedRequest>,
    pub(super) extraction: Option<Extraction>,
    pub(super) outcome: Option<JobOutcome>,
    pub(super) error: Option<String>,
}

impl JobRecord {
    pub(super) fn accepted(
        request_id: String,
        input_digest: String,
        request: PersistedRequest,
    ) -> Self {
        Self {
            version: RECORD_VERSION,
            request_id,
            input_digest,
            state: ExtractionJobState::Accepted,
            request: Some(request),
            extraction: None,
            outcome: None,
            error: None,
        }
    }

    pub(super) fn validate(&self) -> Result<(), ExtractionJobError> {
        self.validate_identity()?;
        if self.has_valid_shape() {
            return Ok(());
        }
        Err(ExtractionJobError::Storage(format!(
            "inconsistent extraction job '{}'",
            self.request_id
        )))
    }

    fn validate_identity(&self) -> Result<(), ExtractionJobError> {
        if self.version != RECORD_VERSION || !valid_digest(&self.request_id) {
            return Err(ExtractionJobError::Storage(
                "invalid extraction job identity".to_owned(),
            ));
        }
        if !valid_digest(&self.input_digest) {
            return Err(ExtractionJobError::Storage(
                "invalid extraction input digest".to_owned(),
            ));
        }
        Ok(())
    }

    fn has_valid_shape(&self) -> bool {
        match self.state {
            ExtractionJobState::Accepted => self.has_accepted_shape(),
            ExtractionJobState::Running => self.has_running_shape(),
            ExtractionJobState::Committed => self.has_committed_shape(),
            ExtractionJobState::Failed => self.has_failed_shape(),
        }
    }

    fn has_accepted_shape(&self) -> bool {
        self.request.is_some()
            && self.extraction.is_none()
            && self.outcome.is_none()
            && self.error.is_none()
    }

    fn has_running_shape(&self) -> bool {
        self.request.is_some() && self.outcome.is_none() && self.error.is_none()
    }

    fn has_committed_shape(&self) -> bool {
        self.request.is_none()
            && self.extraction.is_none()
            && self.outcome.is_some()
            && self.error.is_none()
    }

    fn has_failed_shape(&self) -> bool {
        self.request.is_none()
            && self.extraction.is_none()
            && self.outcome.is_none()
            && self.error.is_some()
    }
}

pub(super) fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}
