//! Tests for the shared transcript bridge — the behaviour the Node, Python
//! and WASM bindings all relay, proven once here instead of once per binding.

use super::*;

fn input(transcript: &str) -> TranscriptCompileInput {
    TranscriptCompileInput {
        query: "deploy pipeline".to_owned(),
        transcript: transcript.to_owned(),
        token_budget: 5000,
        project: None,
        target_model: None,
        policy: None,
        segmentation: None,
    }
}

#[test]
fn test_empty_transcript_is_rejected_by_the_bridge_not_by_the_segmenter() {
    // `segment_transcript` accepts an empty string (a valid, useless,
    // zero-turn input); the guard that turns it into a caller error is this
    // bridge's, and it is what every binding inherits.
    let err = build_transcript_compile_request(input(""))
        .expect_err("an empty transcript must be rejected");
    assert!(
        matches!(err, MemoryError::SegmentationError(ref msg) if msg.contains("empty")),
        "expected a SegmentationError naming the transcript as empty, got {err:?}"
    );
}

#[test]
fn test_plain_transcript_segments_and_wires_the_request() {
    let mut given = input(
        "System: you are a helpful agent.\nUser: what broke the deploy?\n\
         Assistant: clippy failed on main.\n",
    );
    given.project = Some("veles".to_owned());
    let (request, report) =
        build_transcript_compile_request(given).expect("a well-formed plain transcript compiles");

    assert_eq!(request.query, "deploy pipeline");
    assert_eq!(request.token_budget, 5000);
    assert_eq!(request.project.as_deref(), Some("veles"));
    assert!(
        !request.fragments.is_empty(),
        "the transcript must segment into at least one fragment"
    );
    assert!(
        matches!(report.format_detected, SegmentFormat::Plain),
        "a marker-based transcript must detect as plain, got {:?}",
        report.format_detected
    );
    assert_eq!(
        report.segments.len(),
        request.fragments.len(),
        "one audit entry per compiled fragment"
    );
    // The system turn is cache-eligible by default
    // (`SegmentationPolicy::cache_system_turn`) — the role has to survive
    // onto the audit entry so a caller can see why.
    assert_eq!(report.segments[0].role.as_deref(), Some("System"));
}

#[test]
fn test_every_fragment_id_is_a_decimal_string_past_the_float_safe_range() {
    let (_request, report) = build_transcript_compile_request(input(
        "User: what broke the deploy?\nAssistant: clippy failed on main.\n",
    ))
    .expect("a well-formed plain transcript compiles");
    assert!(!report.segments.is_empty());
    for segment in &report.segments {
        let parsed = segment
            .fragment_id
            .parse::<u64>()
            .unwrap_or_else(|_| panic!("fragment_id must be decimal, got {}", segment.fragment_id));
        // Content hashes are 64-bit: rendering them as JSON numbers is what
        // this string form exists to prevent. Assert the *form*, since a
        // given hash may legitimately land below 2^53.
        assert_eq!(segment.fragment_id, parsed.to_string());
    }
}

#[test]
fn test_a_forced_jsonl_policy_is_honoured_and_reported() {
    let mut given = input(
        "{\"role\": \"user\", \"content\": \"what broke the deploy?\"}\n\
         {\"role\": \"assistant\", \"content\": \"clippy failed on main\"}\n",
    );
    given.segmentation = Some(SegmentationPolicy {
        format: SegmentFormat::Jsonl,
        ..SegmentationPolicy::default()
    });
    let (_request, report) =
        build_transcript_compile_request(given).expect("a well-formed jsonl transcript compiles");
    assert!(
        matches!(report.format_detected, SegmentFormat::Jsonl),
        "a forced jsonl policy must report jsonl as detected, got {:?}",
        report.format_detected
    );
}

#[test]
fn test_a_forced_jsonl_parse_failure_surfaces_as_a_segmentation_error() {
    let mut given = input("not jsonl at all");
    given.segmentation = Some(SegmentationPolicy {
        format: SegmentFormat::Jsonl,
        ..SegmentationPolicy::default()
    });
    let err = build_transcript_compile_request(given)
        .expect_err("a forced jsonl format that fails to parse must be a hard error");
    assert!(
        matches!(err, MemoryError::SegmentationError(_)),
        "a FORMAT failure must surface as SegmentationError, not a generic error, got {err:?}"
    );
}

#[test]
fn test_the_report_serializes_to_the_shape_every_binding_publishes() {
    let (_request, report) =
        build_transcript_compile_request(input("User: what broke the deploy?\n"))
            .expect("a well-formed plain transcript compiles");
    let wire = serde_json::to_value(&report).expect("the report is serializable");
    let first = &wire["segments"][0];
    assert!(first["fragment_id"].is_string(), "id crosses as a string");
    for key in ["index", "turn", "kind", "byte_start", "byte_end"] {
        assert!(!first[key].is_null(), "the audit entry must carry `{key}`");
    }
    assert!(!wire["merged_segments"].is_null());
    assert!(!wire["format_detected"].is_null());
}
