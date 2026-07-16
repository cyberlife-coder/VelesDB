//! Unit tests for the deterministic chunker.

use super::*;

fn policy(max: usize, boundary: ChunkBoundary) -> ChunkPolicy {
    ChunkPolicy {
        max_chunk_bytes: max,
        overlap_bytes: 0,
        boundary,
    }
}

#[test]
fn test_chunk_text_empty_input_yields_no_chunks() {
    assert!(chunk_text("", &ChunkPolicy::default()).is_empty());
}

#[test]
fn test_chunk_text_under_ceiling_yields_one_whole_chunk() {
    let text = "short enough";
    let chunks = chunk_text(text, &ChunkPolicy::default());
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].text, text);
    assert_eq!(chunks[0].byte_range, 0..text.len());
    assert_eq!(chunks[0].index, 0);
}

#[test]
fn test_chunk_text_without_overlap_partitions_the_input() {
    let text = "First paragraph about deploys.\n\nSecond paragraph about tests.\n\nThird paragraph about releases.";
    let chunks = chunk_text(text, &policy(40, ChunkBoundary::Paragraph));
    assert!(chunks.len() > 1);
    let rebuilt: String = chunks.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(rebuilt, text, "chunks must reconstruct the input exactly");
    for pair in chunks.windows(2) {
        assert_eq!(
            pair[0].byte_range.end, pair[1].byte_range.start,
            "ranges must be contiguous"
        );
    }
}

#[test]
fn test_chunk_text_prefers_paragraph_boundaries() {
    let text = "Alpha paragraph line.\n\nBeta paragraph line.";
    let chunks = chunk_text(text, &policy(25, ChunkBoundary::Paragraph));
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].text, "Alpha paragraph line.\n\n");
    assert_eq!(chunks[1].text, "Beta paragraph line.");
}

#[test]
fn test_chunk_text_sentence_boundary_cuts_after_enders() {
    let text = "One sentence here. Another one there! A third? Yes.";
    let chunks = chunk_text(text, &policy(20, ChunkBoundary::Sentence));
    assert!(chunks.len() >= 2);
    let rebuilt: String = chunks.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(rebuilt, text);
    assert!(chunks[0].text.starts_with("One sentence here."));
}

#[test]
fn test_chunk_text_never_cuts_inside_a_multibyte_char() {
    // 3-byte chars with a ceiling that is not a multiple of 3.
    let text = "五".repeat(40);
    let chunks = chunk_text(&text, &policy(10, ChunkBoundary::Fixed));
    let rebuilt: String = chunks.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(rebuilt, text);
    for chunk in &chunks {
        assert!(chunk.text.chars().all(|c| c == '五'));
        assert!(chunk.byte_range.len() <= 10);
    }
}

#[test]
fn test_chunk_text_keeps_code_fences_atomic() {
    let fence = "```rust\nlet very_long_binding_name = compute_everything_at_once(1, 2, 3);\n```\n";
    let text = format!("Intro prose before the code.\n\n{fence}\nOutro prose after the code.");
    let chunks = chunk_text(&text, &policy(30, ChunkBoundary::Paragraph));
    let fence_chunk = chunks
        .iter()
        .find(|c| c.text.contains("very_long_binding_name"))
        .expect("the fence must be in some chunk");
    assert!(
        fence_chunk.text.contains("```rust") && fence_chunk.text.trim_end().ends_with("```"),
        "an oversized fence must stay whole, got: {:?}",
        fence_chunk.text
    );
}

#[test]
fn test_chunk_text_hard_splits_an_oversized_paragraph() {
    let text = "x".repeat(100);
    let chunks = chunk_text(&text, &policy(30, ChunkBoundary::Paragraph));
    assert_eq!(chunks.len(), 4);
    assert!(chunks.iter().all(|c| c.byte_range.len() <= 30));
}

#[test]
fn test_chunk_text_overlap_repeats_previous_tail() {
    let text = "abcdefghij".repeat(5);
    let with_overlap = ChunkPolicy {
        max_chunk_bytes: 20,
        overlap_bytes: 5,
        boundary: ChunkBoundary::Fixed,
    };
    let chunks = chunk_text(&text, &with_overlap);
    assert!(chunks.len() > 1);
    for pair in chunks.windows(2) {
        let tail: String = pair[0].text.chars().rev().take(5).collect::<String>();
        let tail: String = tail.chars().rev().collect();
        assert!(
            pair[1].text.starts_with(&tail),
            "each chunk must start with the previous chunk's tail"
        );
    }
}

#[test]
fn test_chunk_text_tiny_ceiling_never_panics_on_multibyte() {
    // A ceiling smaller than one char must advance to the next char
    // boundary, not force a mid-char cut (which would panic on slicing).
    let text = "😀😀😀";
    let chunks = chunk_text(text, &policy(2, ChunkBoundary::Fixed));
    let rebuilt: String = chunks.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(rebuilt, text);
    assert!(chunks.iter().all(|c| !c.text.is_empty()));
}

#[test]
fn test_chunk_text_is_deterministic() {
    let text = "Alpha.\n\nBeta gamma delta. Epsilon!\n\n```code\nblock\n```\nZeta.";
    let policy = ChunkPolicy {
        max_chunk_bytes: 16,
        overlap_bytes: 4,
        boundary: ChunkBoundary::Sentence,
    };
    assert_eq!(chunk_text(text, &policy), chunk_text(text, &policy));
}
