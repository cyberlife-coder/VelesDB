use serde_json::Value;
use velesdb_core::Point;

use crate::embedder::Embedder;
use crate::migration::RawFact;
use crate::storage::NativeStore;
use crate::MemoryError;

pub(super) fn copy_page(
    destination: &NativeStore,
    embedder: &dyn Embedder,
    facts: &[RawFact],
) -> Result<(), MemoryError> {
    let points = facts
        .iter()
        .map(|fact| point_from_raw(fact, embedder))
        .collect::<Result<Vec<_>, _>>()?;
    destination.migration_upsert(points)
}

pub(super) fn sync(
    source: &NativeStore,
    destination: &NativeStore,
    embedder: &dyn Embedder,
    id: u64,
) -> Result<bool, MemoryError> {
    let Some(payload) = source.migration_payload(id)? else {
        destination.migration_delete(id)?;
        return Ok(false);
    };
    let value = Value::Object(payload.clone());
    let content = content(&value, id)?;
    let vector = embedder.embed(content)?;
    destination.migration_upsert(vec![Point::new(id, vector, Some(payload.into()))])?;
    Ok(true)
}

fn point_from_raw(fact: &RawFact, embedder: &dyn Embedder) -> Result<Point, MemoryError> {
    let payload: Value = serde_json::from_str(&fact.payload).map_err(|error| {
        capture(format!(
            "fact {} carries unreadable payload: {error}",
            fact.id
        ))
    })?;
    let vector = embedder.embed(content(&payload, fact.id)?)?;
    Ok(Point::new(fact.id, vector, Some(payload)))
}

fn content(payload: &Value, id: u64) -> Result<&str, MemoryError> {
    payload
        .as_object()
        .and_then(|object| object.get("content"))
        .and_then(Value::as_str)
        .ok_or_else(|| capture(format!("fact {id} has no string content")))
}

fn capture(message: impl Into<String>) -> MemoryError {
    MemoryError::MigrationCapture(message.into())
}
