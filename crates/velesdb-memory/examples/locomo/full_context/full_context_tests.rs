//! Unit tests for the full-context baseline's pure helpers (context assembly,
//! tallying, percentage). The generation/judging paths need a live Ollama and
//! are exercised by the benchmark run itself, not here.

use super::{conversation_context, estimated_tokens, pct, Cell, FULL_CTX_TOKENS};
use crate::dataset::{Sample, Session, Turn};

fn turn(speaker: &str, text: &str) -> Turn {
    Turn {
        speaker: speaker.to_string(),
        dia_id: "D1:1".to_string(),
        text: text.to_string(),
    }
}

fn sample_with(sessions: Vec<Session>) -> Sample {
    Sample {
        sample_id: "conv-test".to_string(),
        sessions,
        qa: Vec::new(),
    }
}

#[test]
fn conversation_context_dates_and_attributes_every_turn_in_order() {
    let sample = sample_with(vec![
        Session {
            index: 0,
            date_time: "1:56 pm on 8 May, 2023".to_string(),
            turns: vec![turn("Alice", "hi"), turn("Bob", "hello")],
        },
        Session {
            index: 1,
            date_time: "9:00 am on 10 June, 2023".to_string(),
            turns: vec![turn("Alice", "back")],
        },
    ]);
    let context = conversation_context(&sample);
    assert_eq!(
        context,
        "- [2023-05-08] Alice: hi\n\
         - [2023-05-08] Bob: hello\n\
         - [2023-06-10] Alice: back"
    );
}

#[test]
fn conversation_context_omits_the_date_prefix_when_the_session_date_is_unparseable() {
    let sample = sample_with(vec![Session {
        index: 0,
        date_time: "sometime".to_string(),
        turns: vec![turn("Alice", "hi")],
    }]);
    assert_eq!(conversation_context(&sample), "- Alice: hi");
}

#[test]
fn pct_guards_a_zero_denominator() {
    assert!((pct(0, 0) - 0.0).abs() < f64::EPSILON);
    assert!((pct(3, 4) - 75.0).abs() < f64::EPSILON);
}

#[test]
fn cell_accuracy_and_mean_tokens_track_recorded_answers() {
    let mut cell = Cell::default();
    cell.add(true, Some(1000));
    cell.add(false, Some(2000));
    cell.add(true, None); // a cache hit: no usage counters
    assert_eq!(cell.n, 3);
    assert_eq!(cell.correct, 2);
    assert!((cell.accuracy() - (200.0 / 3.0)).abs() < 1e-9);
    // Mean over the two live calls only; the cached one is not counted.
    assert_eq!(cell.mean_prompt_tokens(), Some(1500));
}

#[test]
fn cell_mean_tokens_is_none_when_every_answer_was_cached() {
    let mut cell = Cell::default();
    cell.add(true, None);
    assert_eq!(cell.mean_prompt_tokens(), None);
}

#[test]
fn estimated_tokens_over_counts_relative_to_length() {
    assert_eq!(estimated_tokens(""), 0);
    // ~2.5 chars/token: 100 chars -> 40 estimated tokens.
    assert_eq!(estimated_tokens(&"x".repeat(100)), 40);
}

#[test]
fn the_largest_expected_conversation_fits_the_pinned_window() {
    // The biggest LoCoMo conversation is ~92.7k characters; the guard must
    // admit it (estimate + response reserve stays under the window), so a
    // real run never spuriously aborts on the shipped dataset.
    let biggest_chars = "x".repeat(92_716);
    assert!(estimated_tokens(&biggest_chars) + super::RESPONSE_RESERVE_TOKENS < FULL_CTX_TOKENS);
}

#[test]
fn ensure_all_fit_accepts_a_normal_conversation_and_rejects_an_overflowing_one() {
    let small = sample_with(vec![Session {
        index: 0,
        date_time: "1:56 pm on 8 May, 2023".to_string(),
        turns: vec![turn("Alice", "hi")],
    }]);
    let small_ctx = conversation_context(&small);
    assert!(super::ensure_all_fit(&[(&small, small_ctx)]).is_ok());

    // A single turn far larger than the window must be refused up front.
    let huge = sample_with(vec![Session {
        index: 0,
        date_time: "1:56 pm on 8 May, 2023".to_string(),
        turns: vec![turn("Alice", &"word ".repeat(60_000))],
    }]);
    let huge_ctx = conversation_context(&huge);
    assert!(super::ensure_all_fit(&[(&huge, huge_ctx)]).is_err());
}
