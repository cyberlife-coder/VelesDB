//! `LoCoMo` dataset loader.
//!
//! Parses `data/locomo10.json` (snap-research/locomo) into typed [`Sample`]s.
//! The on-disk `conversation` object mixes `speaker_a`/`speaker_b` with
//! `session_<n>` turn-lists and `session_<n>_date_time` strings under dynamic
//! keys, so we read it as a JSON map and lift the sessions out by index.

use std::collections::BTreeMap;
use std::error::Error;
use std::path::Path;

use serde::Deserialize;
use serde_json::{Map, Value};

/// `LoCoMo` question categories, as labelled in the dataset's `category` field.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Category {
    MultiHop,
    Temporal,
    OpenDomain,
    SingleHop,
    Adversarial,
}

impl Category {
    /// Map the dataset's integer code to a category, if recognised.
    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::MultiHop),
            2 => Some(Self::Temporal),
            3 => Some(Self::OpenDomain),
            4 => Some(Self::SingleHop),
            5 => Some(Self::Adversarial),
            _ => None,
        }
    }

    /// Parse a category from its report label (for the `--only` filter).
    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.label() == label)
    }

    /// Short human label for report rows.
    pub fn label(self) -> &'static str {
        match self {
            Self::MultiHop => "multi-hop",
            Self::Temporal => "temporal",
            Self::OpenDomain => "open-domain",
            Self::SingleHop => "single-hop",
            Self::Adversarial => "adversarial",
        }
    }

    /// Adversarial questions are unanswerable: the model should abstain.
    pub fn is_adversarial(self) -> bool {
        matches!(self, Self::Adversarial)
    }

    /// Every category, in report order.
    pub const ALL: [Category; 5] = [
        Self::MultiHop,
        Self::Temporal,
        Self::OpenDomain,
        Self::SingleHop,
        Self::Adversarial,
    ];

    /// Dense index into per-category tallies.
    pub fn index(self) -> usize {
        match self {
            Self::MultiHop => 0,
            Self::Temporal => 1,
            Self::OpenDomain => 2,
            Self::SingleHop => 3,
            Self::Adversarial => 4,
        }
    }
}

/// One dialogue turn, anchored by its `dia_id` (e.g. `"D1:3"`) — the unit the
/// QA `evidence` field points at, and the unit we tag extracted facts with.
#[derive(Clone)]
pub struct Turn {
    pub speaker: String,
    pub dia_id: String,
    pub text: String,
}

/// One conversation session: an ordered turn-list with its timestamp.
pub struct Session {
    pub index: u32,
    pub date_time: String,
    pub turns: Vec<Turn>,
}

impl Session {
    /// The session date as a sortable `YYYYMMDD` integer (the `ColumnStore` key
    /// for temporal filtering), or `0` if the `date_time` cannot be parsed.
    pub fn date_key(&self) -> i64 {
        date_key(&self.date_time)
    }
}

/// Parse a `LoCoMo` `date_time` like `"1:56 pm on 8 May, 2023"` into `YYYYMMDD`.
fn date_key(date_time: &str) -> i64 {
    let Some((_, tail)) = date_time.split_once(" on ") else {
        return 0;
    };
    let cleaned = tail.replace(',', "");
    let parts: Vec<&str> = cleaned.split_whitespace().collect();
    let [day, month, year] = parts.as_slice() else {
        return 0;
    };
    let (Ok(day), Some(month), Ok(year)) =
        (day.parse::<i64>(), month_number(month), year.parse::<i64>())
    else {
        return 0;
    };
    year * 10_000 + month * 100 + day
}

/// Map an English month name to its number (1-12), or `None`.
fn month_number(name: &str) -> Option<i64> {
    const MONTHS: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    let name = name.to_lowercase();
    MONTHS
        .iter()
        .position(|m| *m == name)
        .and_then(|i| i64::try_from(i).ok())
        .map(|i| i + 1)
}

/// One QA probe. `answer` is absent for adversarial items (they carry an
/// `adversarial_answer` instead); `evidence` lists the gold `dia_id`s.
pub struct Qa {
    pub question: String,
    pub answer: Option<String>,
    pub evidence: Vec<String>,
    pub category: Category,
}

/// One `LoCoMo` conversation plus its QA set.
// benchmark harness: field name mirrors dataset JSON key
#[allow(clippy::struct_field_names)]
pub struct Sample {
    pub sample_id: String,
    pub sessions: Vec<Session>,
    pub qa: Vec<Qa>,
}

impl Sample {
    /// Total dialogue turns across all sessions — for run-size reporting.
    pub fn turn_count(&self) -> usize {
        self.sessions.iter().map(|s| s.turns.len()).sum()
    }
}

#[derive(Deserialize)]
struct RawTurn {
    speaker: String,
    dia_id: String,
    text: String,
    #[serde(default)]
    blip_caption: Option<String>,
}

#[derive(Deserialize)]
struct RawQa {
    question: String,
    #[serde(default)]
    answer: Option<Value>,
    #[serde(default)]
    evidence: Vec<String>,
    category: u8,
}

#[derive(Deserialize)]
struct RawSample {
    sample_id: String,
    conversation: Map<String, Value>,
    qa: Vec<RawQa>,
}

/// Load and parse every sample from `path`.
pub fn load(path: &Path) -> Result<Vec<Sample>, Box<dyn Error>> {
    let bytes = std::fs::read(path).map_err(|e| {
        format!(
            "cannot read LoCoMo dataset at {} ({e}). Run examples/locomo/fetch_dataset.sh first.",
            path.display()
        )
    })?;
    let raw: Vec<RawSample> = serde_json::from_slice(&bytes)?;
    raw.into_iter().map(parse_sample).collect()
}

/// Convert one raw sample, lifting `session_<n>` lists out of the dynamic map.
fn parse_sample(raw: RawSample) -> Result<Sample, Box<dyn Error>> {
    let mut sessions: BTreeMap<u32, Session> = BTreeMap::new();
    for (key, value) in &raw.conversation {
        let Some(index) = session_index(key) else {
            continue;
        };
        let turns = parse_turns(value)?;
        // A session always carries its `_date_time` sibling in this dataset; the
        // empty fallback only guards a malformed file and yields a blank date.
        let date_time = raw
            .conversation
            .get(&format!("session_{index}_date_time"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        sessions.insert(
            index,
            Session {
                index,
                date_time,
                turns,
            },
        );
    }
    let qa = raw.qa.into_iter().filter_map(parse_qa).collect();
    Ok(Sample {
        sample_id: raw.sample_id,
        sessions: sessions.into_values().collect(),
        qa,
    })
}

/// `"session_7"` → `Some(7)`; `"session_7_date_time"` / others → `None`.
fn session_index(key: &str) -> Option<u32> {
    let rest = key.strip_prefix("session_")?;
    rest.parse::<u32>().ok()
}

/// Parse a session's turn array, folding image captions into the text so the
/// extractor sees the same content a reader would.
fn parse_turns(value: &Value) -> Result<Vec<Turn>, Box<dyn Error>> {
    let raw: Vec<RawTurn> = serde_json::from_value(value.clone())?;
    Ok(raw
        .into_iter()
        .map(|t| {
            let text = match t.blip_caption {
                Some(cap) if !cap.is_empty() => format!("{} [shared image: {cap}]", t.text),
                _ => t.text,
            };
            Turn {
                speaker: t.speaker,
                dia_id: t.dia_id,
                text,
            }
        })
        .collect())
}

/// Convert a raw QA item, dropping any with an unknown category code.
fn parse_qa(raw: RawQa) -> Option<Qa> {
    let category = Category::from_code(raw.category)?;
    let answer = raw.answer.map(stringify_answer);
    Some(Qa {
        question: raw.question,
        answer,
        evidence: raw.evidence,
        category,
    })
}

/// The `answer` field is usually a string but is occasionally numeric; render
/// both without quotes so judging compares plain text.
fn stringify_answer(value: Value) -> String {
    match value {
        Value::String(s) => s,
        other => other.to_string(),
    }
}
