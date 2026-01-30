//! Tests for snapshot functionality.

#[cfg(test)]
mod tests {
    use super::super::snapshot::*;

    #[test]
    fn test_memory_state_default() {
        let state = MemoryState::default();
        assert!(state.semantic.is_empty());
        assert!(state.episodic.is_empty());
        assert!(state.procedural.is_empty());
        assert!(state.ttl.is_empty());
    }

    #[test]
    fn test_create_and_load_snapshot() {
        let state = MemoryState {
            semantic: vec![1, 2, 3],
            episodic: vec![4, 5, 6],
            procedural: vec![7, 8, 9],
            ttl: vec![10, 11, 12],
        };

        let snapshot_data = create_snapshot(&state);
        let loaded_state = load_snapshot(&snapshot_data).expect("Failed to load snapshot");

        assert_eq!(state.semantic, loaded_state.semantic);
        assert_eq!(state.episodic, loaded_state.episodic);
        assert_eq!(state.procedural, loaded_state.procedural);
        assert_eq!(state.ttl, loaded_state.ttl);
    }

    #[test]
    fn test_snapshot_invalid_magic() {
        let data = vec![
            0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        let result = load_snapshot(&data);
        assert!(matches!(result, Err(SnapshotError::InvalidMagic)));
    }

    #[test]
    fn test_snapshot_checksum_validation() {
        let state = MemoryState::default();
        let mut snapshot_data = create_snapshot(&state);
        if let Some(last) = snapshot_data.last_mut() {
            *last ^= 0xFF;
        }
        let result = load_snapshot(&snapshot_data);
        assert!(matches!(
            result,
            Err(SnapshotError::ChecksumMismatch { .. })
        ));
    }
}
