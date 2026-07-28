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
//! `OllamaExtractor` backend lives behind the `extract` feature.

/// One extracted, graph-ready fact: a self-contained sentence plus the salient
/// topics it concerns. The topics become shared graph hubs, so two facts about
/// the same topic are reachable from one another even with no textual overlap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedFact {
    /// The atomic, standalone fact (pronouns resolved, dates absolute).
    pub text: String,
    /// Salient topics the fact concerns — short canonical lowercase noun
    /// phrases (e.g. `"adoption"`, `"charity race"`). 1-4 is typical.
    pub entities: Vec<String>,
}

/// One extracted entity→entity edge: `subject -[predicate]-> object`.
///
/// Where [`ExtractedFact::entities`] only says "this fact concerns these
/// topics", a relation says *how two topics relate*. It is what turns the
/// bipartite fact↔topic graph into a genuine knowledge graph: from
/// "Julien Lange is Axel Lange's father" the wiring produces the edge
/// `julien lange -[father of]-> axel lange`, so a later walk can answer
/// "who is Axel's father" without any fact mentioning both names again.
///
/// `subject` and `object` are canonicalized exactly like
/// [`ExtractedFact::entities`] (trimmed, lowercased), so they resolve to the
/// SAME entity hub as the topics — the hub id is content-addressed, so this
/// holds across separate calls and across sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedRelation {
    /// Canonical lowercase entity the edge points *from*.
    pub subject: String,
    /// The edge label (e.g. `"father of"`, `"sister of"`, `"works at"`).
    pub predicate: String,
    /// Canonical lowercase entity the edge points *to*.
    pub object: String,
}

/// One extracted entity attribute: `entity.key = value`.
///
/// Attributes are what make "Axel Lange is 15" answerable by a *filter*
/// rather than a similarity search: the pair is merged into the entity hub's
/// `ColumnStore` metadata, so `recall_where` can select on `age >= 15`.
///
/// `value` deliberately keeps its JSON type. `recall_where` comparisons are
/// TYPE-STRICT with no coercion, so an age extracted as the string `"15"`
/// would silently never match a numeric filter — the extraction contract
/// therefore demands numbers stay numbers.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedAttribute {
    /// Canonical lowercase entity the attribute belongs to.
    pub entity: String,
    /// Attribute name, used verbatim as the metadata (`ColumnStore`) field.
    pub key: String,
    /// Attribute value, type preserved (a number stays a JSON number).
    pub value: serde_json::Value,
}

/// Everything one passage yields: the atomic facts, the entity→entity edges
/// between the topics they mention, and the attributes those entities carry.
///
/// [`Extractor::extract`] returns only the `facts` half; a backend that can
/// also read relations and attributes overrides [`Extractor::extract_graph`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Extraction {
    /// The atomic, standalone facts, each with the topics it concerns.
    pub facts: Vec<ExtractedFact>,
    /// Typed edges between entities mentioned in the passage.
    pub relations: Vec<ExtractedRelation>,
    /// Attributes attached to entities mentioned in the passage.
    pub attributes: Vec<ExtractedAttribute>,
}

// --- Orienting a kinship triple stated possessively ---------------------------
//
// A copule ("X est le pere de Y") hangs the relation on its own grammatical
// subject, so subject-of-sentence and subject-of-triple coincide. A possessive
// ("X a une soeur, Y") hangs it on the OTHER one: it is Y who is X's sister.
// Models reliably read the first and reliably mirror the second, and a mirrored
// kinship triple is worse than a missing edge — `entity("Lea Lange")` then
// answers, with the same confidence as any true edge, that Lea is Axel's
// *brother*. The prompt asks for the right direction; this pass guarantees it
// for the construction that gets it wrong, whatever backend produced the triple.

/// Kinship nouns a possessive can introduce, folded (no accents, no ligature)
/// and singular, since the markers below are singular too. Doubling as the
/// predicate whitelist: a triple is only ever re-pointed when its label is one
/// of these, so a non-kinship edge between the same two people is left alone.
const KINSHIP_NOUNS: &[&str] = &[
    "pere",
    "mere",
    "frere",
    "soeur",
    "fils",
    "fille",
    "oncle",
    "tante",
    "cousin",
    "cousine",
    "neveu",
    "niece",
    "grand-pere",
    "grand-mere",
    "beau-pere",
    "belle-mere",
    "demi-frere",
    "demi-soeur",
    "epoux",
    "epouse",
    "mari",
    "femme",
    "father",
    "mother",
    "brother",
    "sister",
    "son",
    "daughter",
    "uncle",
    "aunt",
    "husband",
    "wife",
];

/// What precedes the kinship noun when the sentence hangs the relation on the
/// person it introduces rather than on its own subject. The trailing space is
/// load-bearing: without it `" a un "` would also fire on `"a une"`.
const POSSESSIVE_MARKERS: &[&str] = &[" a un ", " a une ", " a pour ", " has a ", " has an "];

/// Diacritics and ligatures folded to ASCII, so `"sœur"`, `"soeur"` and
/// `"Sœur"` are one token — the passage and the model's label rarely agree on
/// accents, and the whole pass hinges on matching one against the other.
const FOLDINGS: &[(char, &str)] = &[
    ('à', "a"),
    ('â', "a"),
    ('ä', "a"),
    ('é', "e"),
    ('è', "e"),
    ('ê', "e"),
    ('ë', "e"),
    ('î', "i"),
    ('ï', "i"),
    ('ô', "o"),
    ('ö', "o"),
    ('ù', "u"),
    ('û', "u"),
    ('ü', "u"),
    ('ç', "c"),
    ('œ', "oe"),
    ('æ', "ae"),
];

/// Lowercase `text` and fold its diacritics away. Every offset produced from
/// the result indexes the *folded* string, never the original.
fn fold(text: &str) -> String {
    let mut folded = String::with_capacity(text.len());
    for ch in text.chars().flat_map(char::to_lowercase) {
        match FOLDINGS.iter().find(|(from, _)| *from == ch) {
            Some((_, to)) => folded.push_str(to),
            None => folded.push(ch),
        }
    }
    folded
}

/// A possessive construction located in a folded passage. `start`/`end` bracket
/// the kinship noun itself: the person who *has* the relative is named before
/// it, the one it introduces after it.
struct Possessive {
    noun: &'static str,
    start: usize,
    end: usize,
}

/// The earliest possessive construction in `folded`, if any.
fn find_possessive(folded: &str) -> Option<Possessive> {
    POSSESSIVE_MARKERS
        .iter()
        .filter_map(|marker| folded.find(marker).map(|at| at + marker.len()))
        .filter_map(|start| noun_at(folded, start))
        .min_by_key(|possessive| possessive.start)
}

/// The kinship noun sitting at `start`, if the marker introduces one.
fn noun_at(folded: &str, start: usize) -> Option<Possessive> {
    let rest = folded.get(start..)?;
    let noun = KINSHIP_NOUNS
        .iter()
        .find(|noun| starts_with_word(rest, noun))?;
    Some(Possessive {
        noun,
        start,
        end: start + noun.len(),
    })
}

/// `rest` begins with `word` as a whole word, so `"soeurette"` never reads as
/// `"soeur"`.
fn starts_with_word(rest: &str, word: &str) -> bool {
    match rest.strip_prefix(word) {
        Some(tail) => !tail.starts_with(char::is_alphanumeric),
        None => false,
    }
}

/// Every distinct entity the triples name, deduplicated.
fn endpoint_names(relations: &[ExtractedRelation]) -> Vec<String> {
    let mut names: Vec<String> = relations
        .iter()
        .flat_map(|relation| [relation.subject.clone(), relation.object.clone()])
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// The endpoint named closest to the left of the noun: the person who HAS the
/// relative.
fn holder_of(before: &str, names: &[String]) -> Option<String> {
    names
        .iter()
        .filter_map(|name| before.rfind(&fold(name)).map(|at| (at, name)))
        .max_by_key(|(at, _)| *at)
        .map(|(_, name)| name.clone())
}

/// The endpoint the noun introduces: the first one named after it.
fn bearer_of(after: &str, names: &[String]) -> Option<String> {
    names
        .iter()
        .filter_map(|name| after.find(&fold(name)).map(|at| (at, name)))
        .min_by_key(|(at, _)| *at)
        .map(|(_, name)| name.clone())
}

/// The head word of a predicate label, folded: `"sœur de"` → `"soeur"`.
fn predicate_stem(predicate: &str) -> String {
    fold(predicate)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string()
}

/// Whether the triple runs between exactly these two entities, either way round.
fn joins(relation: &ExtractedRelation, one: &str, other: &str) -> bool {
    (relation.subject == one && relation.object == other)
        || (relation.subject == other && relation.object == one)
}

/// Point one triple the way the passage states it.
///
/// The triple built on the noun the passage used belongs to the person that
/// noun introduced; any *other* kinship label over the same pair is its
/// converse and therefore runs the other way. Anything else is untouched.
fn reorient(relation: &mut ExtractedRelation, noun: &str, holder: &str, bearer: &str) {
    let stem = predicate_stem(&relation.predicate);
    if !KINSHIP_NOUNS.contains(&stem.as_str()) || !joins(relation, holder, bearer) {
        return;
    }
    let (subject, object) = if stem == noun {
        (bearer, holder)
    } else {
        (holder, bearer)
    };
    relation.subject = subject.to_string();
    relation.object = object.to_string();
}

/// Re-point the kinship triples a possessive construction states, so the label
/// sits on the person who actually carries it.
///
/// A no-op unless the passage contains a possessive naming a kinship noun AND
/// both sides of it resolve to entities the triples already mention — the pass
/// never invents an edge, never drops one, and never touches a copule.
pub(crate) fn orient_possessive_kinship(passage: &str, relations: &mut [ExtractedRelation]) {
    let folded = fold(passage);
    let Some(possessive) = find_possessive(&folded) else {
        return;
    };
    let names = endpoint_names(relations);
    let Some(holder) = holder_of(&folded[..possessive.start], &names) else {
        return;
    };
    let Some(bearer) = bearer_of(&folded[possessive.end..], &names) else {
        return;
    };
    if holder == bearer {
        return;
    }
    for relation in relations.iter_mut() {
        reorient(relation, possessive.noun, &holder, &bearer);
    }
}

/// Failure produced by an [`Extractor`] backend (e.g. a network-backed model
/// that cannot be reached, or output that cannot be parsed into facts).
#[derive(Debug, thiserror::Error)]
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

// --- Optional batteries-included backend: a local Ollama generative model -----
//
// Enabled with `--features extract`. The default build omits this backend (and
// its HTTP dependency) so the shipped binary stays tiny and fully offline. Like
// the Ollama embedder, it calls a model the user already runs locally, so the
// text never leaves the machine.

/// Default Ollama base URL for the generative extraction endpoint.
#[cfg(feature = "extract")]
pub const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";

/// Per-request timeout. Generation is far slower and more stall-prone than an
/// embedding call, so a wedged model fails the call instead of hanging forever.
#[cfg(feature = "extract")]
const REQUEST_TIMEOUT_SECS: u64 = 300;

/// Ceiling on establishing the TCP connection to Ollama. Short on purpose: a
/// local daemon accepts at once or is not running, and `ureq`'s 30 s default
/// would be paid once per replay.
#[cfg(feature = "extract")]
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Ceiling on writing the request (prompt upload). Unlike the read bound, this
/// one is applied to the socket at connect time and is genuinely in force.
#[cfg(feature = "extract")]
const WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The knobs that actually configure the extractor, named in its failures.
///
/// **Not** the embedder's variables. `main.rs`'s `build_ollama_extractor` reads
/// `VELESDB_MEMORY_EXTRACTOR_URL`/`_MODEL`; telling a user to set
/// `VELESDB_MEMORY_OLLAMA_URL` here would send them to edit a setting this code
/// path never consults — an "actionable" message that is actively wrong. There
/// is no offline fallback to offer either: extraction is opt-in, and running
/// without it is simply not passing an extractor.
#[cfg(feature = "extract")]
const EXTRACT_LEVERS: crate::ollama_retry::OllamaLevers<'static> =
    crate::ollama_retry::OllamaLevers {
        url_var: "VELESDB_MEMORY_EXTRACTOR_URL",
        model_var: "VELESDB_MEMORY_EXTRACTOR_MODEL",
        fallback: None,
    };

/// How one generation attempt failed — transport and body failures may be
/// replayed, a complete response is the server's final word.
#[cfg(feature = "extract")]
enum GenerateCall {
    /// The request never completed. Boxed: `ureq::Error::Status` carries a
    /// whole `Response`.
    Transport(Box<ureq::Error>),
    /// Headers arrived but the body did not read back in full.
    Body(std::io::Error),
}

/// Replay policy for one generation attempt.
#[cfg(feature = "extract")]
fn generate_is_retryable(err: &GenerateCall) -> bool {
    match err {
        GenerateCall::Transport(inner) => crate::ollama_retry::is_retryable(inner),
        GenerateCall::Body(inner) => crate::ollama_retry::io_is_retryable(inner),
    }
}

/// Turn a failed generation into a message that names the endpoint, the model,
/// how many attempts were spent, and the variables that change the outcome.
#[cfg(feature = "extract")]
fn describe_generate_failure(url: &str, model: &str, err: &GenerateCall, attempts: u32) -> String {
    let cause = match err {
        GenerateCall::Transport(inner) => inner.to_string(),
        GenerateCall::Body(inner) => format!("reading the response failed: {inner}"),
    };
    crate::ollama_retry::actionable_failure(
        "generate",
        url,
        model,
        attempts,
        &cause,
        &EXTRACT_LEVERS,
    )
}

/// Extracts facts through a local Ollama `/api/generate` endpoint, keeping the
/// model — and therefore the source text — on the user's own machine.
///
/// The caller picks the generative model (Ollama has no universal default for
/// generation); `temperature` is pinned to `0` and `think` disabled for stable,
/// reproducible output.
#[cfg(feature = "extract")]
#[derive(Debug, Clone)]
pub struct OllamaExtractor {
    base_url: String,
    model: String,
    agent: ureq::Agent,
}

#[cfg(feature = "extract")]
impl OllamaExtractor {
    /// Build an extractor targeting `model` on the Ollama server at `base_url`
    /// (e.g. [`DEFAULT_OLLAMA_URL`]).
    ///
    /// The agent is bounded on four axes, not one. See
    /// [`crate::embedder`]'s `embed_agent` for why `timeout_read` is
    /// subordinate to the global `timeout` in `ureq` and must not be read as a
    /// per-read guarantee; `timeout_connect` and `timeout_write` are the two
    /// that actually bite. The connect bound matters most here: `ureq`'s own
    /// default is 30 s, which for a `localhost` daemon is 15x too long — and
    /// with replays, that idle wait would be paid three times over.
    #[must_use]
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        let timeout = std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS);
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_write(WRITE_TIMEOUT)
            .timeout_read(timeout)
            .timeout(timeout)
            .build();
        Self {
            base_url: base_url.into(),
            model: model.into(),
            agent,
        }
    }
}

#[cfg(feature = "extract")]
impl Extractor for OllamaExtractor {
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        let reply = self.generate(&build_prompt(text))?;
        let raw = json_slice::<Vec<RawFact>>(&reply)
            .ok_or_else(|| ExtractError::Parse(truncate(&reply)))?;
        Ok(raw.into_iter().filter_map(RawFact::into_fact).collect())
    }

    fn extract_graph(&self, text: &str) -> Result<Extraction, ExtractError> {
        let reply = self.generate(&build_graph_prompt(text))?;
        let raw = json_slice_object::<RawExtraction>(&reply)
            .ok_or_else(|| ExtractError::Parse(truncate(&reply)))?;
        Ok(raw.into_extraction())
    }
}

#[cfg(feature = "extract")]
impl OllamaExtractor {
    /// POST one prompt to Ollama's `/api/generate` and return the trimmed reply,
    /// replaying the call when the failure is transient.
    ///
    /// Same defect, same repair as the embedder: this extractor also holds one
    /// `ureq::Agent`, so it also hands out pooled keep-alive connections that
    /// Ollama may have closed, and `ureq` will not replay a POST with a body.
    /// The whole attempt — POST and body read — is inside the closure so a
    /// truncated response is replayed rather than surfacing as a parse error.
    fn generate(&self, prompt: &str) -> Result<String, ExtractError> {
        let url = format!("{}/api/generate", self.base_url);
        let body = serde_json::json!({
            "model": self.model,
            "prompt": prompt,
            "stream": false,
            "think": false,
            // Extraction models are large — the one this crate documents as an
            // example is 21.9 GB — so an unload between calls is the dominant
            // cost, not the generation. Shares the embedder's knob so one
            // setting governs every Ollama call the daemon makes.
            "keep_alive": crate::embedder::keep_alive(),
            "options": { "temperature": 0 },
        })
        .to_string();
        let attempt = || {
            let response = self
                .agent
                .post(&url)
                .set("Content-Type", "application/json")
                .send_string(&body)
                .map_err(|err| GenerateCall::Transport(Box::new(err)))?;
            response.into_string().map_err(GenerateCall::Body)
        };

        let payload = crate::ollama_retry::with_retry(
            &crate::ollama_retry::OLLAMA_RETRIES,
            generate_is_retryable,
            attempt,
        )
        .map_err(|(err, attempts)| {
            ExtractError::Backend(describe_generate_failure(&url, &self.model, &err, attempts))
        })?;
        parse_generate_response(&payload)
    }
}

/// The strict JSON contract the extraction prompt asks the model to honour.
#[cfg(feature = "extract")]
#[derive(serde::Deserialize)]
struct RawFact {
    fact: String,
    #[serde(default)]
    entities: Vec<String>,
}

#[cfg(feature = "extract")]
impl RawFact {
    /// Keep a fact only if it has text; trim and lowercase its topics, dropping
    /// blanks and duplicates so the same topic recurs as the same graph hub.
    fn into_fact(self) -> Option<ExtractedFact> {
        let text = self.fact.trim().to_string();
        if text.is_empty() {
            return None;
        }
        let mut entities: Vec<String> = self
            .entities
            .into_iter()
            .map(|entity| entity.trim().to_lowercase())
            .filter(|entity| !entity.is_empty())
            .collect();
        entities.sort_unstable();
        entities.dedup();
        Some(ExtractedFact { text, entities })
    }
}

/// Canonical form of an entity name: trimmed and lowercased. The one place
/// the rule lives, so a name arriving as a topic, as a relation endpoint, or
/// as an attribute owner always resolves to the SAME entity hub.
#[cfg(feature = "extract")]
fn canonical_entity(name: &str) -> String {
    name.trim().to_lowercase()
}

/// The strict JSON contract the *graph* extraction prompt asks for.
#[cfg(feature = "extract")]
#[derive(serde::Deserialize)]
struct RawExtraction {
    #[serde(default)]
    facts: Vec<RawFact>,
    #[serde(default)]
    relations: Vec<RawRelation>,
    #[serde(default)]
    attributes: Vec<RawAttribute>,
}

#[cfg(feature = "extract")]
#[derive(serde::Deserialize)]
struct RawRelation {
    subject: String,
    predicate: String,
    object: String,
}

#[cfg(feature = "extract")]
#[derive(serde::Deserialize)]
struct RawAttribute {
    entity: String,
    key: String,
    value: serde_json::Value,
}

#[cfg(feature = "extract")]
impl RawExtraction {
    /// Canonicalize and drop the unusable: a relation missing an endpoint or a
    /// label, an attribute missing an owner or a name. A malformed item is
    /// skipped rather than failing the whole passage — one bad triple must not
    /// cost the caller every good fact in the same reply.
    fn into_extraction(self) -> Extraction {
        Extraction {
            facts: self
                .facts
                .into_iter()
                .filter_map(RawFact::into_fact)
                .collect(),
            relations: self
                .relations
                .into_iter()
                .filter_map(RawRelation::into_relation)
                .collect(),
            attributes: self
                .attributes
                .into_iter()
                .filter_map(RawAttribute::into_attribute)
                .collect(),
        }
    }
}

#[cfg(feature = "extract")]
impl RawRelation {
    fn into_relation(self) -> Option<ExtractedRelation> {
        let subject = canonical_entity(&self.subject);
        let object = canonical_entity(&self.object);
        let predicate = self.predicate.trim().to_string();
        // A self-loop carries no information and would sit in the graph as a
        // permanent dead end, so it is dropped alongside the incomplete ones.
        if subject.is_empty() || object.is_empty() || predicate.is_empty() || subject == object {
            return None;
        }
        Some(ExtractedRelation {
            subject,
            predicate,
            object,
        })
    }
}

#[cfg(feature = "extract")]
impl RawAttribute {
    fn into_attribute(self) -> Option<ExtractedAttribute> {
        let entity = canonical_entity(&self.entity);
        let key = self.key.trim().to_string();
        // A null value is the model saying "not stated"; storing it would make
        // an absent attribute look like a known-empty one.
        if entity.is_empty() || key.is_empty() || self.value.is_null() {
            return None;
        }
        Some(ExtractedAttribute {
            entity,
            key,
            value: self.value,
        })
    }
}

/// Build the *graph* extraction prompt: the passage plus a strict JSON
/// contract covering facts, entity→entity edges, and entity attributes.
///
/// The contract insists numbers stay JSON numbers. `recall_where` compares
/// type-strictly, so an age emitted as `"15"` would never match `age >= 15` —
/// no error, just a silent miss, which is the worst possible failure mode for
/// a memory system.
#[cfg(feature = "extract")]
fn build_graph_prompt(text: &str) -> String {
    format!(
        "You are building a knowledge graph from the passage below.\n\n\
Passage:\n{text}\n\n\
Return THREE things.\n\n\
1. \"facts\": the atomic, standalone facts a person would remember. Rewrite each \
as a self-contained sentence (resolve pronouns to names; keep absolute dates). \
For each, list 1-4 key TOPICS it concerns, as short canonical lowercase noun \
phrases, so the same topic recurs as the SAME tag across passages.\n\n\
2. \"relations\": every explicit relationship BETWEEN TWO NAMED ENTITIES, as \
subject/predicate/object triples. Use the entity's full name, lowercase \
(e.g. \"julien lange\").\n\
The predicate is a LABEL, not a sentence: **at most 3 words**, lowercase, in \
the passage's own language (e.g. \"pere de\", \"soeur de\", \"works at\", \
\"moteur de recherche\"). NEVER restate the sentence — write \"surveille les \
fuites\", not \"est utilise pour la surveillance de fuites de donnees\". If you \
cannot say it in 3 words, pick the closest short label.\n\
State the triple in the direction the passage states it, and add the converse \
ONLY if the passage states it too.\n\
DIRECTION: the subject is whoever CARRIES the relation, not the subject of the \
sentence. \"A a une soeur, B\" means B is A's sister, so the triple is \
B/\"soeur de\"/A — never A/\"soeur de\"/B. Same for every possessive \
(\"a un frere\", \"a une fille\", \"has a brother\").\n\
Every named entity the passage RELATES to another must appear in at least one \
triple — an entity that only receives attributes and no edge is a dead end in \
the graph.\n\n\
3. \"attributes\": every property a named entity HAS, as entity/key/value. Use \
short lowercase keys (\"age\", \"ville\", \"employeur\"). Emit numbers as JSON \
NUMBERS, never strings: 15, not \"15\". Omit anything the passage does not state.\n\n\
Return ONLY this JSON object, no prose:\n\
{{\"facts\": [{{\"fact\": string, \"entities\": [string]}}], \
\"relations\": [{{\"subject\": string, \"predicate\": string, \"object\": string}}], \
\"attributes\": [{{\"entity\": string, \"key\": string, \"value\": string|number|boolean}}]}}"
    )
}

/// Build the extraction prompt: the passage plus a strict JSON contract.
#[cfg(feature = "extract")]
fn build_prompt(text: &str) -> String {
    format!(
        "You are building a memory graph from the passage below.\n\n\
Passage:\n{text}\n\n\
Extract the atomic, standalone facts a person would remember. Rewrite each as a \
self-contained sentence (resolve pronouns to names; keep absolute dates). For \
each fact also list 1-4 key TOPICS it concerns: the recurring subjects, \
activities, events, interests, plans, places, organisations, or named people a \
later question might reference. Use short, canonical, lowercase noun phrases \
(e.g. \"adoption\", \"charity race\", \"therapy\", \"new job\") so the same topic \
recurs as the SAME tag across passages.\n\n\
Return ONLY a JSON array, no prose, each item exactly:\n\
{{\"fact\": string, \"entities\": [string]}}"
    )
}

/// Pull the `response` string out of Ollama's `/api/generate` JSON envelope.
#[cfg(feature = "extract")]
fn parse_generate_response(body: &str) -> Result<String, ExtractError> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|err| ExtractError::Backend(format!("invalid generate response: {err}")))?;
    let text = value
        .get("response")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ExtractError::Backend("ollama reply had no `response` field".to_string()))?;
    Ok(text.trim().to_string())
}

/// A short, single-line preview of model output for error messages.
#[cfg(feature = "extract")]
fn truncate(text: &str) -> String {
    const LIMIT: usize = 120;
    let mut out = String::new();
    for word in text.split_whitespace() {
        // Check the budget *before* pushing so we never need a post-hoc
        // `String::truncate`, which would panic if the limit fell mid-UTF-8 char.
        let sep_len = usize::from(!out.is_empty());
        if out.len() + sep_len + word.len() > LIMIT {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// Parse `text` into `T`, first slicing out the outermost JSON array/object.
/// Local models usually honour "return only JSON" but occasionally wrap it in
/// fences or a sentence; slicing the first balanced span tolerates that.
#[cfg(feature = "extract")]
fn json_slice<T: serde::de::DeserializeOwned>(text: &str) -> Option<T> {
    let slice = balanced_slice(text)?;
    serde_json::from_str::<T>(slice).ok()
}

/// [`json_slice`] for a reply whose top level is a JSON **object**.
///
/// The array-preferring form cannot be reused: the graph reply is
/// `{"facts": [...], ...}`, whose first `[` belongs to a *nested* field, so
/// preferring arrays slices out the inner facts list and then fails to read it
/// as the whole extraction. That failure is invisible to a stub-backed test —
/// only a real model reply goes through this path.
#[cfg(feature = "extract")]
fn json_slice_object<T: serde::de::DeserializeOwned>(text: &str) -> Option<T> {
    let slice = balanced_slice_preferring(text, b'{')?;
    serde_json::from_str::<T>(slice).ok()
}

/// Return the substring spanning the first balanced `[..]` or `{..}`, honouring
/// string literals and escapes so brackets inside quotes don't miscount.
///
/// Prefers an array: the fact-only reply is a JSON list, and prose before it
/// ("Result {ok}: [...]") may carry a stray `{` that would mis-slice the
/// object span instead of the array.
#[cfg(feature = "extract")]
fn balanced_slice(text: &str) -> Option<&str> {
    balanced_slice_preferring(text, b'[')
}

/// [`balanced_slice`] with the caller choosing which delimiter wins when both
/// appear — the shape the caller actually expects at the top level.
#[cfg(feature = "extract")]
fn balanced_slice_preferring(text: &str, preferred: u8) -> Option<&str> {
    let bytes = text.as_bytes();
    let fallback = if preferred == b'[' { b'{' } else { b'[' };
    let start = bytes
        .iter()
        .position(|&b| b == preferred)
        .or_else(|| bytes.iter().position(|&b| b == fallback))?;
    let open = bytes[start];
    let close = if open == b'[' { b']' } else { b'}' };
    let mut depth = 0u32;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, &byte) in bytes[start..].iter().enumerate() {
        if in_string {
            in_string = step_string(&mut escaped, byte);
        } else if scan_structural(byte, open, close, &mut in_string, &mut depth) {
            return Some(&text[start..=start + offset]);
        }
    }
    None
}

/// Advance the structural scan for one out-of-string byte; returns `true` once
/// the outermost bracket has just closed (`depth` back to zero).
#[cfg(feature = "extract")]
fn scan_structural(byte: u8, open: u8, close: u8, in_string: &mut bool, depth: &mut u32) -> bool {
    if byte == b'"' {
        *in_string = true;
    } else if byte == open {
        *depth += 1;
    } else if byte == close {
        *depth = depth.saturating_sub(1);
        return *depth == 0;
    }
    false
}

/// Advance the in-string escape state for one byte; returns whether the scanner
/// is still inside the string literal afterwards.
#[cfg(feature = "extract")]
fn step_string(escaped: &mut bool, byte: u8) -> bool {
    match (*escaped, byte) {
        (true, _) => {
            *escaped = false;
            true
        }
        (false, b'\\') => {
            *escaped = true;
            true
        }
        (false, b'"') => false,
        (false, _) => true,
    }
}

#[cfg(all(test, feature = "extract"))]
mod tests {
    use super::*;

    /// Regression: the graph reply is an OBJECT whose first `[` belongs to the
    /// nested `facts` field. Slicing with the array preference grabbed that
    /// inner list and failed to read it as the whole extraction — a real model
    /// reply was rejected wholesale while every stub-backed test stayed green.
    #[test]
    fn parses_a_graph_reply_whose_first_bracket_is_nested() {
        let reply = r#"{ "facts": [ { "fact": "Zephyrin is the father of Kaltar.", "entities": ["zephyrin", "kaltar"] } ], "relations": [ { "subject": "zephyrin", "predicate": "pere de", "object": "kaltar" } ], "attributes": [ { "entity": "kaltar", "key": "age", "value": 15 } ] }"#;
        let raw: RawExtraction = json_slice_object(reply).expect("the object is sliced whole");
        let extraction = raw.into_extraction();
        assert_eq!(extraction.facts.len(), 1);
        assert_eq!(extraction.relations.len(), 1);
        assert_eq!(extraction.relations[0].predicate, "pere de");
        assert_eq!(extraction.attributes.len(), 1);
        assert_eq!(extraction.attributes[0].value, serde_json::json!(15));
    }

    /// Prose (and a fenced block) around the object must not defeat slicing.
    #[test]
    fn parses_a_graph_reply_wrapped_in_prose_and_fences() {
        let reply = "Here you go:\n```json\n{\"facts\": [], \"relations\": [{\"subject\": \"a\", \"predicate\": \"knows\", \"object\": \"b\"}], \"attributes\": []}\n```";
        let raw: RawExtraction = json_slice_object(reply).expect("sliced past the fence");
        assert_eq!(raw.into_extraction().relations.len(), 1);
    }

    /// The fact-only path must keep preferring an array: prose carrying a stray
    /// `{` before the list is exactly what that preference exists to survive.
    #[test]
    fn fact_only_slicing_still_prefers_the_array() {
        let reply = "Result {ok}: [{\"fact\": \"A ships B.\", \"entities\": [\"b\"]}]";
        let raw: Vec<RawFact> = json_slice(reply).expect("array sliced despite the stray brace");
        assert_eq!(raw.len(), 1);
    }

    #[test]
    fn graph_prompt_demands_numeric_values_and_the_three_sections() {
        let prompt = build_graph_prompt("Kaltar a 15 ans.");
        assert!(prompt.contains("Kaltar a 15 ans."));
        assert!(prompt.contains("\"relations\""));
        assert!(prompt.contains("\"attributes\""));
        assert!(prompt.contains("15, not \"15\""));
    }

    #[test]
    fn prompt_carries_the_passage_and_json_contract() {
        let prompt = build_prompt("Alice adopted a dog in 2021.");
        assert!(prompt.contains("Alice adopted a dog in 2021."));
        assert!(prompt.contains("\"fact\": string"));
    }

    /// The graph prompt has to bound the predicate explicitly. Asking for "a
    /// short label" was not enough: on real content the model answered
    /// "est utilise pour la surveillance de fuites de donnees" — a restated
    /// sentence, which makes the edge unreadable in `entity()`.
    #[test]
    fn graph_prompt_bounds_the_predicate_and_demands_edges() {
        let prompt = build_graph_prompt("Ahmia is an onion search engine.");
        assert!(prompt.contains("Ahmia is an onion search engine."));
        assert!(
            prompt.contains("at most 3 words"),
            "the predicate length must be a hard bound, not a suggestion"
        );
        assert!(
            prompt.contains("NEVER restate the sentence"),
            "the counter-example is what stops a restated sentence"
        );
        assert!(
            prompt.contains("at least one triple"),
            "an entity with attributes but no edge is a dead end — the prompt \
             must ask for the edge"
        );
    }

    /// The prompt must state the possessive rule explicitly: asked only to
    /// "state the triple in the direction the passage states it", the model
    /// read the grammatical subject as the subject of the triple and mirrored
    /// every possessive.
    #[test]
    fn graph_prompt_states_which_side_carries_the_relation() {
        let prompt = build_graph_prompt("Axel Lange a une soeur, Lea Lange.");
        assert!(
            prompt.contains("whoever CARRIES the relation"),
            "the rule must name the carrier, not just \"the direction\""
        );
        assert!(
            prompt.contains("never A/\"soeur de\"/B"),
            "the counter-example is what makes the rule unambiguous"
        );
    }

    #[test]
    fn parses_facts_from_a_fenced_reply() {
        let reply = "Sure!\n```json\n[{\"fact\":\"Alice adopted a dog.\",\"entities\":[\"Adoption\",\"adoption\",\"\"]}]\n```";
        let facts: Vec<RawFact> = json_slice(reply).expect("slice json");
        let facts: Vec<ExtractedFact> = facts.into_iter().filter_map(RawFact::into_fact).collect();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].text, "Alice adopted a dog.");
        // Trimmed, lowercased, deduplicated, blanks dropped.
        assert_eq!(facts[0].entities, vec!["adoption".to_string()]);
    }

    #[test]
    fn drops_a_textless_fact() {
        let raw = RawFact {
            fact: "   ".to_string(),
            entities: vec!["x".to_string()],
        };
        assert!(raw.into_fact().is_none());
    }

    #[test]
    fn parses_response_envelope() {
        let text = parse_generate_response(r#"{"response":"  [] "}"#).expect("parse");
        assert_eq!(text, "[]");
    }

    #[test]
    fn rejects_response_without_field() {
        assert!(matches!(
            parse_generate_response(r#"{"oops":true}"#),
            Err(ExtractError::Backend(_))
        ));
    }
}
