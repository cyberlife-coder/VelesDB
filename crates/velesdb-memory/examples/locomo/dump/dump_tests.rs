use std::io::{BufReader, Read as _};

use super::*;
use crate::dataset::{Category, Qa};

fn fact(
    id: u64,
    ts: i64,
    score: f64,
    graph_weight: f64,
    dia_id: &str,
    text: &str,
) -> RetrievedFact {
    RetrievedFact {
        id,
        text: text.to_string(),
        dia_ids: vec![dia_id.to_string()],
        score,
        graph_weight,
        ts,
    }
}

fn qa(question: &str, answer: Option<&str>, evidence: &[&str], category: Category) -> Qa {
    Qa {
        question: question.to_string(),
        answer: answer.map(str::to_string),
        evidence: evidence.iter().map(|s| (*s).to_string()).collect(),
        category,
    }
}

#[test]
fn ymd_to_ordinal_spans_known_dates() {
    let start = ymd_to_ordinal(20_230_101).unwrap();
    let end = ymd_to_ordinal(20_230_131).unwrap();
    assert_eq!(end - start, 30);
    // Leap day: 2024-02-29 exists, 2023 has no such date but is never asked for.
    let feb28 = ymd_to_ordinal(20_240_228).unwrap();
    let feb29 = ymd_to_ordinal(20_240_229).unwrap();
    let mar01 = ymd_to_ordinal(20_240_301).unwrap();
    assert_eq!(feb29 - feb28, 1);
    assert_eq!(mar01 - feb29, 1);
}

#[test]
fn ymd_to_ordinal_rejects_bad_month_or_day() {
    assert!(ymd_to_ordinal(20_231_301).is_none()); // month 13
    assert!(ymd_to_ordinal(20_230_100).is_none()); // day 0
}

#[test]
fn estimate_tokens_counts_whitespace_words() {
    assert_eq!(estimate_tokens("a b  c\nd"), 4);
    assert_eq!(estimate_tokens(""), 0);
}

#[test]
fn to_fact_records_marks_evidence_hit_per_fact() {
    let facts = vec![
        fact(1, 20_230_101, 0.9, 0.0, "D1:1", "on topic"),
        fact(2, 20_230_102, 0.1, 0.5, "D1:9", "distractor"),
    ];
    let evidence = vec!["D1:1".to_string()];
    let records = to_fact_records(&facts, &evidence);
    assert_eq!(records.len(), 2);
    assert!(records[0].evidence_hit);
    assert!(!records[1].evidence_hit);
    assert_eq!(records[0].rank, 0);
    assert_eq!(records[1].rank, 1);
}

#[test]
fn date_span_ignores_unknown_dates_and_dedups() {
    let facts = vec![
        fact(1, 20_230_101, 0.0, 0.0, "D1:1", "a"),
        fact(2, 20_230_101, 0.0, 0.0, "D1:2", "b"), // same date, dedup
        fact(3, 20_230_111, 0.0, 0.0, "D1:3", "c"),
        fact(4, 0, 0.0, 0.0, "D1:4", "d"), // unknown, excluded
    ];
    let records = to_fact_records(&facts, &[]);
    let (n_distinct, span) = date_span(&records);
    assert_eq!(n_distinct, 2);
    assert_eq!(span, 10);
}

#[test]
fn write_record_round_trips_through_jsonl() {
    let dir = std::env::temp_dir().join(format!("velesdb-locomo-dump-test-{}", std::process::id()));
    let path = write_sample_record(&dir);

    let value = read_first_record(&path);
    assert_eq!(value["conversation_id"], "conv-7");
    assert_eq!(value["question_idx"], 3);
    assert_eq!(value["graph_on"], true);
    assert_eq!(value["category"], "temporal");
    assert_eq!(value["predicted"], "2023");
    assert_eq!(value["prompt_tokens"], 120);
    assert_eq!(value["completion_tokens"], 8);
    assert_eq!(value["raw_facts"].as_array().unwrap().len(), 1);
    assert_eq!(value["reranked_facts"].as_array().unwrap().len(), 1);
    assert_eq!(value["reranked_facts"][0]["evidence_hit"], true);
    assert_eq!(value["raw_facts"][0]["evidence_hit"], false);

    std::fs::remove_dir_all(&dir).ok();
}

/// Write one fixture record to `dir/out.jsonl` via the real `write_record`
/// path (not a hand-built JSON string), returning the file path.
fn write_sample_record(dir: &std::path::Path) -> std::path::PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join("out.jsonl");
    let mut sink = DumpSink::create(&path).unwrap();

    let raw = vec![fact(1, 20_230_101, 0.9, 0.0, "D1:1", "raw only")];
    let reranked = vec![fact(2, 20_230_102, 0.8, 0.2, "D1:2", "made the cut")];
    let question = qa(
        "when did it happen",
        Some("2023"),
        &["D1:2"],
        Category::Temporal,
    );
    write_record(
        QuestionTrace {
            conversation_id: "conv-7",
            question_idx: 3,
            sink: &mut sink,
        },
        &RecordInputs {
            qa: &question,
            cfg: test_cfg(),
            graph_on: true,
            raw: &raw,
            reranked: &reranked,
            candidate: "2023",
            usage: Some(TokenUsage {
                prompt: 120,
                completion: 8,
            }),
            correct: true,
            f1: Some(1.0),
            evidence_hit: true,
            date_on: true,
            scaffold_on: false,
            is_temporal_trigger: true,
            latest_ts: 20_230_102,
        },
    )
    .unwrap();
    drop(sink);
    path
}

/// Read back the sole JSONL line at `path` as a parsed value.
fn read_first_record(path: &std::path::Path) -> serde_json::Value {
    let mut contents = String::new();
    BufReader::new(std::fs::File::open(path).unwrap())
        .read_to_string(&mut contents)
        .unwrap();
    let lines: Vec<&str> = contents.lines().collect();
    assert_eq!(lines.len(), 1, "one record per write_record call");
    serde_json::from_str(lines[0]).unwrap()
}

fn test_cfg() -> EvalCfg {
    EvalCfg {
        k: 8,
        graph_boost: 0.15,
        hops: 2,
        multihop_only: false,
        idf_weight: false,
        seed_breadth: 1,
        date_context: true,
        date_routed: true,
        temporal_scaffold: false,
        cot: false,
        bm25: false,
        claude_judge: false,
        claude_gen: false,
    }
}
