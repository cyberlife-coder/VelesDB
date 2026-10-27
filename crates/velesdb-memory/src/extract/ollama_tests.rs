use super::*;

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
