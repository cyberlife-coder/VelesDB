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

/// A plugin-local condition core already codes carries core's code and
/// recoverability, read from the matching core variant rather than a copy.
#[test]
fn test_command_error_codes_come_from_core() {
    use velesdb_core::Error as Core;
    let io = || std::io::Error::other("x");
    let cases = vec![
        (
            Error::CollectionNotFound("x".to_string()),
            Core::CollectionNotFound(String::new()),
        ),
        (Error::NotFound("x".to_string()), Core::PointNotFound(0)),
        (
            Error::DimensionMismatch {
                expected: 4,
                actual: 2,
            },
            Core::DimensionMismatch {
                expected: 4,
                actual: 2,
            },
        ),
        (
            Error::InvalidConfig("x".to_string()),
            Core::Config(String::new()),
        ),
        (
            Error::Serialization("x".to_string()),
            Core::Serialization(String::new()),
        ),
        (Error::Io(io()), Core::Io(io())),
    ];

    for (err, core) in cases {
        let label = format!("{err:?}");
        let cmd_err: CommandError = err.into();
        assert_eq!(cmd_err.code, core.code(), "{label}");
        assert_eq!(cmd_err.recoverable, core.is_recoverable(), "{label}");
    }
}

#[test]
fn test_command_error_pins_the_taxonomy_codes() {
    let cases = vec![
        (Error::NotFound("x".to_string()), "VELES-003"),
        (
            Error::DimensionMismatch {
                expected: 4,
                actual: 2,
            },
            "VELES-004",
        ),
        (Error::InvalidConfig("x".to_string()), "VELES-009"),
        (Error::Serialization("x".to_string()), "VELES-012"),
    ];

    for (err, expected_code) in cases {
        let cmd_err: CommandError = err.into();
        assert_eq!(cmd_err.code, expected_code);
    }
}

#[test]
fn test_command_error_config_load_is_a_config_error() {
    let err = Error::ConfigLoad {
        path: "velesdb.toml".to_string(),
        source: velesdb_core::config::ConfigError::FileNotFound("velesdb.toml".to_string()),
    };

    let cmd_err: CommandError = err.into();

    assert_eq!(cmd_err.code, "VELES-009");
    assert!(cmd_err.recoverable);
}

#[test]
fn test_command_error_task_join_is_plugin_only_and_not_recoverable() {
    let cmd_err: CommandError = Error::TaskJoin("panicked".to_string()).into();

    assert_eq!(cmd_err.code, "TASK_JOIN");
    assert!(!cmd_err.recoverable);
}

#[test]
fn test_command_error_database_forwards_core_recoverability() {
    let core_err = velesdb_core::Error::IndexCorrupted("bad header".to_string());

    let cmd_err: CommandError = Error::Database(core_err).into();

    assert_eq!(cmd_err.code, "VELES-008");
    assert!(!cmd_err.recoverable);
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
    assert_eq!(cmd_err.code, "VELES-003");
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
    assert_eq!(cmd_err.code, "VELES-004");
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
