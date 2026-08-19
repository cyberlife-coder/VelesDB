use super::*;

#[test]
fn test_payload_creation() {
    let payload = UpdateCheckPayload::new("abc123".to_string(), "core");

    assert!(!payload.version.is_empty());
    assert!(!payload.os.is_empty());
    assert!(!payload.arch.is_empty());
    assert_eq!(payload.instance_hash, "abc123");
    assert_eq!(payload.edition, "core");
}

#[test]
fn test_payload_serialization() {
    let payload = UpdateCheckPayload::new("abc123".to_string(), "core");
    let json = serde_json::to_string(&payload).expect("Failed to serialize");

    assert!(json.contains("\"os\""));
    assert!(json.contains("\"arch\""));
    assert!(json.contains("\"instance_hash\":\"abc123\""));
    assert!(json.contains("\"edition\":\"core\""));
}
