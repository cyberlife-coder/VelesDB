use crate::storage::NativeStore;
use crate::MemoryError;

pub(super) fn stores_match(
    source: &NativeStore,
    destination: &NativeStore,
    batch: usize,
    edge_cap: usize,
) -> Result<(), MemoryError> {
    let mut cursor = None;
    loop {
        let (source_page, source_next) = source.migration_list(cursor, batch)?;
        let (destination_page, destination_next) = destination.migration_list(cursor, batch)?;
        compare_facts(&source_page, &destination_page)?;
        compare_edges(source, destination, &source_page, edge_cap)?;
        if source_next != destination_next {
            return Err(capture("source and destination cursors diverged"));
        }
        let Some(next) = source_next else {
            return Ok(());
        };
        cursor = Some(next);
    }
}

fn compare_facts(
    source: &[crate::migration::RawFact],
    destination: &[crate::migration::RawFact],
) -> Result<(), MemoryError> {
    if source.len() != destination.len() {
        return Err(capture("source and destination fact counts diverged"));
    }
    for (source, destination) in source.iter().zip(destination) {
        if source.id != destination.id || source.payload != destination.payload {
            return Err(capture(format!(
                "source and destination fact {} diverged",
                source.id
            )));
        }
    }
    Ok(())
}

fn compare_edges(
    source: &NativeStore,
    destination: &NativeStore,
    facts: &[crate::migration::RawFact],
    edge_cap: usize,
) -> Result<(), MemoryError> {
    for fact in facts {
        let mut expected = source.migration_live_edges(fact.id, edge_cap)?;
        let mut actual = destination.migration_live_edges(fact.id, edge_cap)?;
        expected.sort_by_key(velesdb_core::GraphEdge::id);
        actual.sort_by_key(velesdb_core::GraphEdge::id);
        if expected != actual {
            return Err(capture(format!(
                "source and destination outgoing edges for {} diverged",
                fact.id
            )));
        }
    }
    Ok(())
}

fn capture(message: impl Into<String>) -> MemoryError {
    MemoryError::MigrationCapture(message.into())
}
