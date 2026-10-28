use velesdb_core::{AnyCollection, GraphEdge, Point};

use super::NativeStore;
use crate::migration::RawFact;
use crate::MemoryError;

impl NativeStore {
    pub(crate) fn migration_list(
        &self,
        cursor: Option<u64>,
        limit: usize,
    ) -> Result<(Vec<RawFact>, Option<u64>), MemoryError> {
        crate::migration::scroll_page(&self.db, self.collection_name(), cursor, limit)
    }

    pub(crate) fn migration_payload(
        &self,
        id: u64,
    ) -> Result<Option<crate::Metadata>, MemoryError> {
        self.memory
            .semantic()
            .get_metadata(id)
            .map_err(MemoryError::from)
    }

    pub(crate) fn migration_contains(&self, id: u64) -> Result<bool, MemoryError> {
        Ok(self
            .migration_collection()?
            .get(&[id])
            .into_iter()
            .next()
            .flatten()
            .is_some())
    }

    pub(crate) fn migration_upsert(&self, points: Vec<Point>) -> Result<(), MemoryError> {
        if points.is_empty() {
            return Ok(());
        }
        self.migration_collection()?.upsert(points)?;
        Ok(())
    }

    pub(crate) fn migration_delete(&self, id: u64) -> Result<(), MemoryError> {
        let collection = self.migration_collection()?;
        let AnyCollection::Vector(collection) = collection else {
            return Err(capture("semantic memory is not a vector collection"));
        };
        collection.delete(&[id])?;
        Ok(())
    }

    pub(crate) fn migration_live_edges(
        &self,
        from: u64,
        cap: usize,
    ) -> Result<Vec<GraphEdge>, MemoryError> {
        let bounded = self.memory.semantic().relations_bounded(from, cap)?;
        if bounded.truncated {
            return Err(degree_error(from, cap));
        }
        validate_edges(from, &bounded.edges)?;
        Ok(bounded.edges)
    }

    pub(crate) fn migration_replace_edges(
        &self,
        from: u64,
        edges: &[GraphEdge],
        cap: usize,
    ) -> Result<(), MemoryError> {
        validate_edges(from, edges)?;
        let current = self.current_edges_bounded(from, cap)?;
        self.remove_edges(current)?;
        self.add_edges(edges)
    }

    fn remove_edges(&self, edges: Vec<GraphEdge>) -> Result<(), MemoryError> {
        for edge in edges {
            if !self.memory.semantic().unrelate(edge.id())? {
                return Err(capture(format!(
                    "could not durably remove edge {}",
                    edge.id()
                )));
            }
        }
        Ok(())
    }

    fn add_edges(&self, edges: &[GraphEdge]) -> Result<(), MemoryError> {
        let collection = self.migration_collection()?;
        for edge in edges {
            collection.add_edge(edge.clone())?;
        }
        Ok(())
    }

    fn current_edges_bounded(&self, from: u64, cap: usize) -> Result<Vec<GraphEdge>, MemoryError> {
        let bounded = self.memory.semantic().relations_bounded(from, cap)?;
        if bounded.truncated {
            return Err(degree_error(from, cap));
        }
        Ok(self.migration_collection()?.get_outgoing_edges(from))
    }

    fn migration_collection(&self) -> Result<AnyCollection, MemoryError> {
        self.db
            .get_any_collection(self.collection_name())
            .ok_or_else(|| capture("semantic memory collection is absent"))
    }

    fn collection_name(&self) -> &str {
        self.memory.semantic().collection_name()
    }
}

fn validate_edges(from: u64, edges: &[GraphEdge]) -> Result<(), MemoryError> {
    for edge in edges {
        let derived = velesdb_core::hash_edge_id(edge.source(), edge.target(), edge.label());
        if edge.source() != from || edge.id() != derived {
            return Err(capture(format!(
                "edge {} is not a derived outgoing edge of {from}",
                edge.id()
            )));
        }
    }
    Ok(())
}

fn degree_error(from: u64, cap: usize) -> MemoryError {
    capture(format!(
        "outgoing degree of {from} exceeds migration cap {cap}"
    ))
}

fn capture(message: impl Into<String>) -> MemoryError {
    MemoryError::MigrationCapture(message.into())
}
