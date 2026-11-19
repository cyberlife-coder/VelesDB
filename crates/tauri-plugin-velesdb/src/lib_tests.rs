use super::*;

#[test]
fn test_velesdb_state_creation() {
    // Arrange
    let path = std::path::PathBuf::from("/tmp/test");

    // Act
    let state = VelesDbState::new(path.clone());

    // Assert
    assert_eq!(state.path(), &path);
}

#[test]
fn test_get_app_data_dir_structure() {
    // Act
    let path = get_app_data_dir("test-app").unwrap();

    // Assert - path should end with test-app/velesdb
    assert!(path.ends_with("test-app/velesdb") || path.ends_with("test-app\\velesdb"));
    assert!(path.to_string_lossy().contains("test-app"));
}

#[test]
fn test_get_app_data_dir_different_apps() {
    // Act
    let path1 = get_app_data_dir("app1").unwrap();
    let path2 = get_app_data_dir("app2").unwrap();

    // Assert - different apps should have different paths
    assert_ne!(path1, path2);
}

/// The builder records the agent-memory dimension, defaulting to core's.
#[test]
fn the_builder_records_the_memory_dimension() {
    let default = crate::Builder::new("./unused");
    assert_eq!(
        default.memory_dimension(),
        velesdb_core::agent::DEFAULT_DIMENSION
    );

    let configured = crate::Builder::new("./unused").with_memory_dimension(768);
    assert_eq!(configured.memory_dimension(), 768);
}
