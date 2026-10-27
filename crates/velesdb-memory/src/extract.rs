//! Optional text → facts + entities extraction, the layer that makes the graph
//! self-build.
//!
//! The Agent Memory SDK is *bring-your-own-links*: [`crate::MemoryService::remember`]
//! only stores the links the caller supplies, so a graph is only ever as rich as
//! what the caller wires by hand. This module adds the missing commodity on top:
//! an [`Extractor`] turns a paragraph of raw text into atomic facts, each tagged
//! with the salient topics it mentions. [`crate::MemoryService::remember_extracted`]
//! then stores those facts and wires the fact↔entity graph automatically, so
//! `why()` has something to traverse without any manual `relate()`.
//!
//! Mirroring the [`crate::embedder`] pattern, the plug-point is dependency-free
//! (bring your own LLM by implementing [`Extractor`]) while a batteries-included
//! `OllamaExtractor` backend lives behind the `extractor-http` feature.

mod kinship;
mod model;
#[cfg(feature = "extractor-http")]
mod ollama;
#[cfg(feature = "extractor-http")]
mod openai_extractor;
mod outline;
#[cfg(feature = "extractor-http")]
mod prompt;
#[cfg(feature = "extractor-http")]
mod reply;
mod selection;
#[cfg(feature = "extractor-http")]
mod wire;

pub(crate) use kinship::orient_kinship;
pub use model::{ExtractedAttribute, ExtractedFact, ExtractedRelation, Extraction};
#[cfg(feature = "extractor-http")]
pub use ollama::{OllamaExtractor, DEFAULT_OLLAMA_URL};
#[cfg(feature = "extractor-http")]
pub use openai_extractor::OpenAiExtractor;
pub use outline::OutlineExtractor;
pub use selection::{select_extractor, ExtractorSelection};

/// Failure produced by an [`Extractor`] backend (e.g. a network-backed model
/// that cannot be reached, or output that cannot be parsed into facts).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive] // error enum, grows by nature; matching externally requires a wildcard arm
pub enum ExtractError {
    /// The extraction backend (network, subprocess, …) returned an error.
    #[error("extraction backend error: {0}")]
    Backend(String),
    /// The backend produced output that could not be parsed into facts.
    #[error("could not parse facts from extractor output: {0}")]
    Parse(String),
}

/// Turns a passage of raw text into atomic, graph-ready facts.
///
/// Implement this to plug in any model — a local LLM, a hosted API, or a
/// deterministic rule set — and feed the result straight into
/// [`crate::MemoryService::remember_extracted`].
pub trait Extractor {
    /// Extract the atomic facts a reader would remember from `text`.
    ///
    /// # Errors
    /// Returns [`ExtractError`] if the backend fails or its output cannot be
    /// parsed into facts.
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, ExtractError>;

    /// Extract the facts *and* the entity→entity edges and entity attributes
    /// the passage states.
    ///
    /// Defaults to [`Self::extract`] with no relations and no attributes, so
    /// every backend written against the fact-only contract keeps compiling
    /// and keeps working — it simply builds the bipartite fact↔topic graph it
    /// always did. A backend that can read structure overrides this.
    ///
    /// # Errors
    /// Returns [`ExtractError`] if the backend fails or its output cannot be
    /// parsed.
    fn extract_graph(&self, text: &str) -> Result<Extraction, ExtractError> {
        Ok(Extraction {
            facts: self.extract(text)?,
            ..Extraction::default()
        })
    }
}

/// Forward [`Extractor`] through an [`Arc`](std::sync::Arc), so a shared `Arc<dyn Extractor>`
/// (e.g. one held by the MCP server) satisfies the `X: Extractor` bound on
/// [`crate::MemoryService::remember_extracted`].
///
/// Both methods are forwarded. Forwarding only `extract` would silently route
/// every `Arc`-held backend — which is *every* backend the MCP server and the
/// bindings use — through the fact-only default, discarding the relations and
/// attributes the inner extractor actually produced.
impl<T: Extractor + ?Sized> Extractor for std::sync::Arc<T> {
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        (**self).extract(text)
    }

    fn extract_graph(&self, text: &str) -> Result<Extraction, ExtractError> {
        (**self).extract_graph(text)
    }
}

/// A shared, object-safe extractor. The MCP server and the language bindings
/// hold one of these (an `Option`), so the extraction tool can be attached at
/// runtime without the type being generic.
pub type DynExtractor = std::sync::Arc<dyn Extractor + Send + Sync>;
