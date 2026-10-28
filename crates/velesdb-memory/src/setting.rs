//! Why a startup setting was refused: a backend name this build does not know,
//! or an environment variable that is missing, blank or malformed.
//!
//! These used to be bare messages, handed to every caller as a `String` that
//! each adapter then turned into its own invalid-argument error. They are the
//! caller's to correct and never a server fault, and the variants below name
//! which setting is wrong so a binding can say so without parsing prose.
//!
//! The payload stays prose where an operator reads it, for the reason
//! [`crate::MemoryError`] gives: nothing outside this crate matches on it.

/// A startup setting that cannot be used as given.
///
/// `non_exhaustive`: a new backend or variable adds a variant without a
/// breaking release.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SettingError {
    /// The embedding backend name is not one this build offers.
    #[error(
        "unknown embedding backend '{name}' (expected 'hash' for the \
         offline deterministic embedder, 'ollama' for a local model, or \
         'openai' for any OpenAI-compatible server — oMLX, llama.cpp, LM \
         Studio, vLLM or a hosted provider, selected by URL rather than by \
         name)"
    )]
    UnknownEmbedder {
        /// What was asked for.
        name: String,
    },

    /// The extraction backend name is not one this build offers.
    #[error(
        "unknown extraction backend '{name}' (expected 'outline' for the \
         offline deterministic reader, 'ollama' for a local generative \
         model, 'openai' for any OpenAI-compatible server — oMLX, \
         llama.cpp, LM Studio, vLLM or a hosted provider, selected by URL \
         rather than by name — or 'none')"
    )]
    UnknownExtractor {
        /// What was asked for.
        name: String,
    },

    /// An `openai` role has no server URL, and there is no default for one.
    #[error(
        "{role}=openai requires {role}_URL — the server's origin and port, \
         no path (e.g. http://localhost:8020). There is no default: `openai` \
         is a protocol, and only you know which server speaks it here."
    )]
    MissingUrl {
        /// The role's variable prefix, e.g. `VELESDB_MEMORY_EXTRACTOR`.
        role: String,
    },

    /// An `openai` role has no model identifier.
    #[error("{role}=openai requires {role}_MODEL — the model identifier the server expects")]
    MissingModel {
        /// The role's variable prefix, e.g. `VELESDB_MEMORY_EMBEDDER`.
        role: String,
    },

    /// An API token variable is set to an empty or blank value, which is not
    /// the same as unset.
    #[error(
        "{variable} is set but empty — unset it entirely to send no credential. An \
         empty token would go out as `Authorization: Bearer `, which a server \
         rejects as a bad credential rather than a missing one."
    )]
    BlankToken {
        /// The variable that holds the token.
        variable: String,
    },

    /// A disabled extractor (`none`) was offered where one that extracts is
    /// required, such as the daemon-level default.
    #[error("a disabled extractor cannot be attached as the default")]
    ExtractionDisabled,

    /// The extractor's chat-template arguments are set but are not a JSON
    /// object. The reason is the parser's position message, or the kind of
    /// value found — never the raw text, which may hold a secret the chat
    /// template reads.
    #[cfg(feature = "extractor-http")]
    #[error("{variable} must be a JSON object: {reason}")]
    InvalidChatTemplateKwargs {
        /// The variable that holds the arguments.
        variable: String,
        /// Why the value is not an object.
        reason: String,
    },

    /// The configured recall quality is not one the search engine knows.
    #[cfg(feature = "persistence")]
    #[error("{detail}")]
    InvalidSearchQuality {
        /// The search engine's own wording: the value, and the accepted forms.
        detail: String,
    },
}
