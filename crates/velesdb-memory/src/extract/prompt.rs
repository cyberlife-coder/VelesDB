//! The extraction prompts, shared by every generative backend.

/// Build the *graph* extraction prompt: the passage plus a strict JSON
/// contract covering facts, entity→entity edges, and entity attributes.
///
/// The contract insists numbers stay JSON numbers. `recall_where` compares
/// type-strictly, so an age emitted as `"15"` would never match `age >= 15` —
/// no error, just a silent miss, which is the worst possible failure mode for
/// a memory system.
pub(super) fn build_graph_prompt(text: &str) -> String {
    format!(
        "You are building a knowledge graph from the passage below.\n\n\
Passage:\n{text}\n\n\
STEP 0 — Identify the passage's language. Everything you write (facts, \
predicates, attribute keys) MUST be in THAT language. Do not copy the language \
of the examples below: they are shown in several languages on purpose, and you \
must match the PASSAGE, never the example.\n\n\
Return THREE things.\n\n\
1. \"facts\": the atomic, standalone facts a person would remember, in the \
passage's language. Rewrite each as a self-contained sentence (resolve \
pronouns to names; keep absolute dates). For each, list 1-4 key TOPICS it \
concerns, as short canonical lowercase noun phrases, so the same topic recurs \
as the SAME tag across passages.\n\n\
2. \"relations\": every explicit relationship BETWEEN TWO NAMED ENTITIES, as \
subject/predicate/object triples. Use the entity's full name, lowercase \
(e.g. \"bruno durand\").\n\
The predicate is a LABEL, not a sentence: **at most 3 words**, lowercase, in \
the passage's language. Examples of the SHAPE, each in its own language — \
match the passage, not these: a French passage gives \"travaille chez\", \
\"pere de\"; an English passage gives \"works at\", \"father of\"; a Spanish \
passage gives \"trabaja en\". NEVER restate the sentence — write \"surveille \
les fuites\", not \"est utilise pour la surveillance de fuites de donnees\". \
If you cannot say it in 3 words, pick the closest short label.\n\
DIRECTION: the subject is whoever CARRIES the relation, not the subject of the \
sentence. \"A a une soeur, B\" means B is A's sister, so the triple is \
B/\"soeur de\"/A — never A/\"soeur de\"/B. \"A has a brother, B\" means B is \
A's brother: B/\"brother of\"/A. Same for every possessive.\n\
Never emit both directions of the SAME predicate over the same pair — \
\"X brother of Y\" plus \"Y brother of X\" is a contradiction, not a \
converse: emit exactly one. But two DIFFERENT predicates the passage states \
separately over the same pair (\"A possede B\" then \"B appartient a A\") \
are two stated facts — keep both.\n\
Every named entity the passage RELATES to another must appear in at least one \
triple — an entity that only receives attributes and no edge is a dead end in \
the graph.\n\n\
3. \"attributes\": every property a named entity HAS, as entity/key/value. Use \
short lowercase keys in the passage's language (\"age\", \"ville\", \
\"employeur\"). Emit numbers as JSON NUMBERS, never strings: 15, not \"15\". \
Omit anything the passage does not state.\n\n\
Return ONLY this JSON object, no prose, no markdown fence:\n\
{{\"facts\": [{{\"fact\": string, \"entities\": [string]}}], \
\"relations\": [{{\"subject\": string, \"predicate\": string, \"object\": string}}], \
\"attributes\": [{{\"entity\": string, \"key\": string, \"value\": string|number|boolean}}]}}"
    )
}

/// Build the extraction prompt: the passage plus a strict JSON contract.
pub(super) fn build_prompt(text: &str) -> String {
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

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;
