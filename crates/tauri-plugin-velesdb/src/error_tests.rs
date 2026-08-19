use super::*;

#[test]
fn test_error_display_collection_not_found() {
    // Arrange
    let err = Error::CollectionNotFound("test_collection".to_string());

    // Act
    let message = err.to_string();

    // Assert
    assert_eq!(message, "Collection 'test_collection' not found");
}

#[test]
fn test_error_display_invalid_config() {
    // Arrange
    let err = Error::InvalidConfig("missing dimension".to_string());

    // Act
    let message = err.to_string();

    // Assert
    assert_eq!(message, "Invalid configuration: missing dimension");
}

#[test]
fn test_command_error_from_error() {
    // Arrange
    let err = Error::CollectionNotFound("docs".to_string());

    // Act
    let cmd_err: CommandError = err.into();

    // Assert — uses VELES-XXX code from core
    assert_eq!(cmd_err.code, "VELES-002");
    assert!(cmd_err.message.contains("docs"));
}

#[test]
fn test_command_error_codes() {
    // Arrange & Act & Assert
    let cases = vec![
        (Error::CollectionNotFound("x".to_string()), "VELES-002"),
        (Error::InvalidConfig("x".to_string()), "INVALID_CONFIG"),
        (Error::Serialization("x".to_string()), "SERIALIZATION_ERROR"),
    ];

    for (err, expected_code) in cases {
        let cmd_err: CommandError = err.into();
        assert_eq!(cmd_err.code, expected_code);
    }
}

#[test]
fn test_command_error_database_uses_core_code() {
    // Arrange — wrap a core error (CollectionExists uses VELES-001)
    let core_err = velesdb_core::Error::CollectionExists("test".to_string());
    let err = Error::Database(core_err);

    // Act
    let cmd_err: CommandError = err.into();

    // Assert — should forward the core VELES-XXX code
    assert_eq!(cmd_err.code, "VELES-001");
}

#[test]
fn test_agent_error_preserves_not_found() {
    // Arrange — agent NotFound must not be flattened to InvalidConfig
    let agent_err = velesdb_core::agent::AgentMemoryError::NotFound("proc 7".to_string());

    // Act
    let err: Error = agent_err.into();
    let cmd_err: CommandError = err.into();

    // Assert
    assert_eq!(cmd_err.code, "NOT_FOUND");
    assert!(cmd_err.message.contains("proc 7"));
}

#[test]
fn test_agent_error_preserves_dimension_mismatch() {
    // Arrange
    let agent_err = velesdb_core::agent::AgentMemoryError::DimensionMismatch {
        expected: 384,
        actual: 128,
    };

    // Act
    let err: Error = agent_err.into();
    let cmd_err: CommandError = err.into();

    // Assert
    assert_eq!(cmd_err.code, "DIMENSION_MISMATCH");
    assert!(cmd_err.message.contains("384"));
    assert!(cmd_err.message.contains("128"));
}

#[test]
fn test_agent_error_database_forwards_core_code() {
    // Arrange — DatabaseError must surface the core VELES-XXX code
    let core_err = velesdb_core::Error::CollectionExists("m".to_string());
    let agent_err = velesdb_core::agent::AgentMemoryError::DatabaseError(core_err);

    // Act
    let err: Error = agent_err.into();
    let cmd_err: CommandError = err.into();

    // Assert
    assert_eq!(cmd_err.code, "VELES-001");
}

#[test]
fn test_command_error_io_uses_veles_011() {
    // Arrange
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file gone");
    let err = Error::Io(io_err);

    // Act
    let cmd_err: CommandError = err.into();

    // Assert — IO maps to VELES-011
    assert_eq!(cmd_err.code, "VELES-011");
}
