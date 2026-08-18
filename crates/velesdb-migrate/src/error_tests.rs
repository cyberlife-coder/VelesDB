use super::*;

#[test]
fn test_error_display() {
    let err = Error::Config("missing API key".to_string());
    assert_eq!(err.to_string(), "Configuration error: missing API key");
}

#[test]
fn test_error_from_io() {
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
    let err: Error = io_err.into();
    assert!(matches!(err, Error::Io(_)));
}
