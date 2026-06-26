//! The extraction layer velesdb-memory intentionally does not ship.
//!
//! A local LLM reads each session and returns atomic facts, every fact tagged
//! with the source `dia_id`s (so retrieval can be scored against the gold
//! `evidence`) and the salient entities it mentions (so ingestion can wire the
//! fact↔entity graph that `why()` later traverses). Honesty note: the benchmark
//! score reflects this extractor as much as the database.

use std::collections::HashSet;
use std::error::Error;

use serde::Deserialize;

use crate::dataset::{Sample, Session};
use crate::ollama_gen::Generator;
use crate::parse::json_slice;

/// One extracted, graph-ready fact.
#[derive(Clone)]
pub struct Fact {
    pub text: String,
    pub dia_ids: Vec<String>,
    pub entities: Vec<String>,
    /// Session date as a sortable `YYYYMMDD` key (the `ColumnStore` facet).
    pub ts: i64,
}

#[derive(Deserialize)]
struct RawFact {
    fact: String,
    #[serde(default)]
    dia_ids: Vec<String>,
    #[serde(default)]
    entities: Vec<String>,
}

/// Extract facts for every session of `sample`, in order. Cached per session by
/// the generator, so re-runs and resumes cost no GPU.
pub fn extract_sample(generator: &Generator, sample: &Sample) -> Result<Vec<Fact>, Box<dyn Error>> {
    let speakers = speaker_names(sample);
    let mut facts = Vec::new();
    for session in &sample.sessions {
        facts.extend(extract_session(
            generator,
            &sample.sample_id,
            session,
            &speakers,
        )?);
    }
    Ok(facts)
}

/// The lowercased set of speaker names in the conversation. They are excluded as
/// graph entities: as the two ever-present participants they would otherwise
/// become mega-hubs linking almost every fact, so a traversal through them would
/// reach "all facts about this person" and drown the multi-hop signal in noise.
fn speaker_names(sample: &Sample) -> HashSet<String> {
    sample
        .sessions
        .iter()
        .flat_map(|s| s.turns.iter())
        .map(|t| t.speaker.to_lowercase())
        .collect()
}

/// Extract and sanitise the facts of a single session.
fn extract_session(
    generator: &Generator,
    sample_id: &str,
    session: &Session,
    speakers: &HashSet<String>,
) -> Result<Vec<Fact>, Box<dyn Error>> {
    if session.turns.is_empty() {
        return Ok(Vec::new());
    }
    let valid_ids: HashSet<&str> = session.turns.iter().map(|t| t.dia_id.as_str()).collect();
    let prompt = build_prompt(sample_id, session, speakers);
    let reply = generator.generate(&prompt)?;
    let Some(raw) = json_slice::<Vec<RawFact>>(&reply) else {
        // Surface the drop rather than silently undercounting the graph.
        eprintln!(
            "        warning: {sample_id} session {} returned unparseable JSON; 0 facts",
            session.index
        );
        return Ok(Vec::new());
    };
    let ts = session.date_key();
    Ok(raw
        .into_iter()
        .filter_map(|r| sanitise(r, &valid_ids, speakers, ts))
        .collect())
}

/// Keep a fact only if it has text and at least one real, in-session `dia_id`;
/// drop hallucinated ids, speaker-name entities, and entities that collapse to
/// nothing. Tags the fact with its session timestamp `ts`.
fn sanitise(
    raw: RawFact,
    valid_ids: &HashSet<&str>,
    speakers: &HashSet<String>,
    ts: i64,
) -> Option<Fact> {
    let text = raw.fact.trim().to_string();
    if text.is_empty() {
        return None;
    }
    let dia_ids: Vec<String> = raw
        .dia_ids
        .into_iter()
        .filter(|id| valid_ids.contains(id.as_str()))
        .collect();
    if dia_ids.is_empty() {
        return None;
    }
    let mut seen = HashSet::new();
    let entities: Vec<String> = raw
        .entities
        .into_iter()
        .map(|e| e.trim().to_lowercase())
        .filter(|e| e.len() >= 3 && !speakers.contains(e) && seen.insert(e.clone()))
        .collect();
    Some(Fact {
        text,
        dia_ids,
        entities,
        ts,
    })
}

/// Build the extraction prompt: numbered dialogue plus a strict JSON contract.
fn build_prompt(sample_id: &str, session: &Session, speakers: &HashSet<String>) -> String {
    use std::fmt::Write as _;
    let mut dialogue = String::new();
    for turn in &session.turns {
        // Writing to a String is infallible; the result is intentionally ignored.
        let _ = writeln!(
            dialogue,
            "[{}] {}: {}",
            turn.dia_id, turn.speaker, turn.text
        );
    }
    let mut names: Vec<&str> = speakers.iter().map(String::as_str).collect();
    names.sort_unstable();
    format!(
        "You are building a memory graph from a conversation (sample {sample_id}, \
session {idx}, dated {date}). The speakers are {names}.\n\n\
Dialogue (each line is [dia_id] speaker: text):\n{dialogue}\n\
Extract the atomic, standalone facts a person would remember. Rewrite each as a \
self-contained sentence (resolve pronouns to names; keep absolute dates). For \
each fact also list 1-4 key TOPICS it concerns: the recurring subjects, \
activities, events, interests, plans, places, organisations, or named people \
OTHER than the speakers that a later question might reference. Use short, \
canonical, lowercase noun phrases (e.g. \"adoption\", \"charity race\", \
\"therapy\", \"new job\") so the same topic recurs as the SAME tag across \
sessions. Never use the speakers' own names or generic filler as a topic.\n\n\
Return ONLY a JSON array, no prose, each item exactly:\n\
{{\"fact\": string, \"dia_ids\": [string], \"entities\": [string]}}",
        idx = session.index,
        date = session.date_time,
        names = names.join(" and "),
    )
}
