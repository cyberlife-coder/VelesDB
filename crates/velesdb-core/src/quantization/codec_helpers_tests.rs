use super::*;

#[test]
fn test_serialize_with_header_concatenates_header_and_payload() {
    let header = [0x01, 0x02, 0x03, 0x04];
    let payload = [0xAA, 0xBB, 0xCC];
    let result = serialize_with_header(&header, &payload);
    assert_eq!(result, vec![0x01, 0x02, 0x03, 0x04, 0xAA, 0xBB, 0xCC]);
}

#[test]
fn test_serialize_with_header_empty_payload() {
    let header = [0x01, 0x02];
    let payload: [u8; 0] = [];
    let result = serialize_with_header(&header, &payload);
    assert_eq!(result, vec![0x01, 0x02]);
}

#[test]
fn test_serialize_with_header_empty_header() {
    let header: [u8; 0] = [];
    let payload = [0xAA, 0xBB];
    let result = serialize_with_header(&header, &payload);
    assert_eq!(result, vec![0xAA, 0xBB]);
}

#[test]
fn test_validate_and_split_header_valid() {
    let bytes = [0x01, 0x02, 0x03, 0x04, 0xAA, 0xBB];
    let (header, payload) = validate_and_split_header(&bytes, 4, "Test").unwrap();
    assert_eq!(header, &[0x01, 0x02, 0x03, 0x04]);
    assert_eq!(payload, &[0xAA, 0xBB]);
}

#[test]
fn test_validate_and_split_header_exact_size() {
    let bytes = [0x01, 0x02, 0x03, 0x04];
    let (header, payload) = validate_and_split_header(&bytes, 4, "Test").unwrap();
    assert_eq!(header, &[0x01, 0x02, 0x03, 0x04]);
    assert!(payload.is_empty());
}

#[test]
fn test_validate_and_split_header_too_short() {
    let bytes = [0x01, 0x02];
    let err = validate_and_split_header(&bytes, 4, "SomeType").unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert!(
        err.to_string().contains("SomeType"),
        "Error message should contain the type name"
    );
}

#[test]
fn test_validate_and_split_header_empty_input() {
    let bytes: [u8; 0] = [];
    let err = validate_and_split_header(&bytes, 1, "Empty").unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}
