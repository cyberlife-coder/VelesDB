//! The value types an extraction produces: facts, relations, attributes.

/// One extracted, graph-ready fact: a self-contained sentence plus the salient
/// topics it concerns. The topics become shared graph hubs, so two facts about
/// the same topic are reachable from one another even with no textual overlap.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
/// "Bruno Durand is Theo Durand's father" the wiring produces the edge
/// `bruno durand -[father of]-> theo durand`, so a later walk can answer
/// "who is Theo's father" without any fact mentioning both names again.
///
/// `subject` and `object` are canonicalized exactly like
/// [`ExtractedFact::entities`] (trimmed, lowercased), so they resolve to the
/// SAME entity hub as the topics — the hub id is content-addressed, so this
/// holds across separate calls and across sessions.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
/// Attributes are what make "Theo Durand is 15" answerable by a *filter*
/// rather than a similarity search: the pair is merged into the entity hub's
/// `ColumnStore` metadata, so `recall_where` can select on `age >= 15`.
///
/// `value` deliberately keeps its JSON type. `recall_where` comparisons are
/// TYPE-STRICT with no coercion, so an age extracted as the string `"15"`
/// would silently never match a numeric filter — the extraction contract
/// therefore demands numbers stay numbers.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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
/// [`super::Extractor::extract`] returns only the `facts` half; a backend that can
/// also read relations and attributes overrides [`super::Extractor::extract_graph`].
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Extraction {
    /// The atomic, standalone facts, each with the topics it concerns.
    pub facts: Vec<ExtractedFact>,
    /// Typed edges between entities mentioned in the passage.
    pub relations: Vec<ExtractedRelation>,
    /// Attributes attached to entities mentioned in the passage.
    pub attributes: Vec<ExtractedAttribute>,
}
