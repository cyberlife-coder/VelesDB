use serde_json::Value;

/// Collections `AgentMemory` opens, all at the same dimension — so any one of
/// them refusing the new dimension makes the whole store unopenable, and an
/// inventory that skipped the empty ones would under-report the work.
pub const AGENT_COLLECTIONS: &[&str] =
    &["_semantic_memory", "_episodic_memory", "_procedural_memory"];

/// One fact as the rebuild will need to re-create it.
///
/// The old vector is deliberately absent: it is recomputed by the new
/// embedder, and carrying it would only invite writing it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawFact {
    /// The id the fact must keep. Not a suggestion.
    pub id: u64,
    /// The whole stored payload, reserved keys included — `content` and every
    /// `_veles_*` key travel here verbatim.
    pub payload: String,
}

/// Read every fact of `collection` out of `db`, in pages of `page`.
///
/// `ORDER BY id` is load-bearing, not decoration. Measured on 2026-08-03: the
/// natural scan order is NOT id order — a seeded store answered
/// `[1, 2, 3, 100, 4, 5, 6, 7]` — so an `OFFSET` walk over the unordered scan
/// pages by *position* in a layout the engine is free to rearrange
/// (`reorder_for_locality` does exactly that). Ordering first makes each page
/// boundary a value.
///
/// This is only sound because the walk runs under the exclusive store lock with
/// nothing else writing.
///
/// Prefer [`enumerate_by_cursor`] for a rebuild. This walk is kept as the
/// independent second path the cursor is checked against — two routes through
/// the engine agreeing is evidence, where a cursor compared against a list this
/// module built would only prove self-consistency — and it is bounded at
/// `100_000` facts, so it is not the one to migrate a real store with.
///
/// # Errors
/// Returns [`crate::MemoryError`] if the scan cannot be parsed or executed.
pub fn enumerate_collection(
    db: &velesdb_core::Database,
    collection: &str,
    page: usize,
) -> Result<Vec<RawFact>, crate::MemoryError> {
    let mut out: Vec<RawFact> = Vec::new();
    let mut offset = 0usize;
    loop {
        let batch = enumerate_page(db, collection, page, offset)?;
        if batch.is_empty() {
            break;
        }
        let returned = batch.len();
        out.extend(batch);
        if returned < page {
            break;
        }
        offset += page;
    }
    Ok(out)
}

/// One page of `collection`, starting at `offset` — the unit a checkpoint
/// resumes from, and what makes the walk above interruptible rather than
/// all-or-nothing.
///
/// # Errors
/// Returns [`crate::MemoryError`] if the scan cannot be parsed or executed.
pub fn enumerate_page(
    db: &velesdb_core::Database,
    collection: &str,
    page: usize,
    offset: usize,
) -> Result<Vec<RawFact>, crate::MemoryError> {
    let sql = format!("SELECT * FROM {collection} ORDER BY id LIMIT {page} OFFSET {offset}");
    let query = velesdb_core::velesql::Parser::parse(&sql)
        .map_err(|e| velesdb_core::Error::Query(e.to_string()))?;
    let hits = db.execute_query(&query, &std::collections::HashMap::new())?;
    Ok(hits
        .into_iter()
        .map(|hit| RawFact {
            id: hit.point.id,
            payload: hit
                .point
                .payload
                .as_ref()
                .map_or_else(|| Value::Null.to_string(), std::string::ToString::to_string),
        })
        .collect())
}

/// One batch of `collection`, starting strictly after `cursor`, with the cursor
/// to resume from.
///
/// This is the enumeration the rebuild should use, and it is NOT the `VelesQL`
/// walk above. The engine already carries a cursor primitive — `scroll_batch`
/// on the collection itself, keyed on the point id, exclusive, ascending — and
/// it bypasses the query pipeline entirely. That matters twice over:
///
/// * The query pipeline asks the collection for `limit + offset` rows and
///   clamps the total to `MAX_LIMIT` (`100_000`), so an `OFFSET` walk goes blind
///   past that mark. A cursor never accumulates an offset, so it has no such
///   bound.
/// * `WHERE id > n` really is unavailable — filters read the payload and the id
///   is not in it — but that only ever ruled out expressing the cursor *in
///   `VelesQL`*. It never ruled out the cursor.
///
/// Returns the batch and the cursor to pass next; `None` means the collection
/// is exhausted.
///
/// # Errors
/// Returns [`crate::MemoryError`] if the collection is absent, is of a kind
/// that does not scroll, or if the scroll itself fails.
pub fn scroll_page(
    db: &velesdb_core::Database,
    collection: &str,
    cursor: Option<u64>,
    batch: usize,
) -> Result<(Vec<RawFact>, Option<u64>), crate::MemoryError> {
    let any = db.get_any_collection(collection).ok_or_else(|| {
        velesdb_core::Error::Query(format!("collection `{collection}` not found"))
    })?;
    let scrolled = match &any {
        velesdb_core::AnyCollection::Vector(c) => c.scroll_batch(cursor, batch, None),
        velesdb_core::AnyCollection::Graph(c) => c.scroll_batch(cursor, batch, None),
        velesdb_core::AnyCollection::Metadata(c) => c.scroll_batch(cursor, batch, None),
        _ => {
            return Err(velesdb_core::Error::Query(format!(
                "collection `{collection}` is of a kind that does not scroll"
            ))
            .into())
        }
    }?;
    let facts = scrolled
        .points
        .into_iter()
        .map(|point| RawFact {
            id: point.id,
            payload: point
                .payload
                .as_ref()
                .map_or_else(|| Value::Null.to_string(), std::string::ToString::to_string),
        })
        .collect();
    Ok((facts, scrolled.next_cursor))
}

/// Read every fact of `collection` out of `db` by cursor, in batches of `batch`.
///
/// Termination is on an exhausted cursor or an empty batch — never on a SHORT
/// one. `scroll_batch` keeps scanning candidate ids until it has `batch_size`
/// live points or the ids run out, and it skips TTL-expired points on the way,
/// so a short batch does mean the end; but the two stop conditions are cheap and
/// the walk should not depend on that internal detail holding.
///
/// Skipping the expired is the behaviour this rebuild wants, not a quirk to work
/// around: the contract says an export must exclude already-expired facts so the
/// rebuild cannot resurrect them.
///
/// # Errors
/// Returns [`crate::MemoryError`] if any batch fails.
pub fn enumerate_by_cursor(
    db: &velesdb_core::Database,
    collection: &str,
    batch: usize,
) -> Result<Vec<RawFact>, crate::MemoryError> {
    let mut out: Vec<RawFact> = Vec::new();
    let mut cursor: Option<u64> = None;
    loop {
        let (facts, next) = scroll_page(db, collection, cursor, batch)?;
        if facts.is_empty() {
            break;
        }
        out.extend(facts);
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// PUTTING A FACT BACK
//
// Reading every fact out proves half of the feasibility question. The other
// half is whether it goes back UNCHANGED — same id, same payload, same absolute
// expiry — into a destination the new embedder sized. That is proven here, on a
// destination, and never on the source.
// ---------------------------------------------------------------------------

/// What putting a fact back produced.
///
/// A collision is a RESULT, not an error and emphatically not a silent
/// overwrite: `upsert` would replace whatever sat under that id without a
/// word, and a rebuild that did so would destroy the very fact it was
/// preserving. The caller decides; this reports.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Reinsertion {
    /// The id was free, and the fact now occupies it.
    Inserted,
    /// The id was taken. NOTHING was written.
    Collision {
        /// The payload already stored under that id, left exactly as it was.
        existing: String,
    },
}

/// Put `fact` back into `collection` under its ORIGINAL id, with `vector` as
/// the new embedder computed it.
///
/// The old vector is not carried and not written: it belongs to the model being
/// migrated away from, and re-writing it would produce a store whose vectors
/// and whose recorded model disagree.
///
/// # Errors
/// Returns [`crate::MemoryError`] if the collection is absent, if the stored
/// payload is not readable, or if the write fails.
pub fn reinsert(
    db: &velesdb_core::Database,
    collection: &str,
    fact: &RawFact,
    vector: &[f32],
) -> Result<Reinsertion, crate::MemoryError> {
    let any = db.get_any_collection(collection).ok_or_else(|| {
        velesdb_core::Error::Query(format!("collection `{collection}` not found"))
    })?;
    if let Some(Some(existing)) = any.get(&[fact.id]).into_iter().next() {
        return Ok(Reinsertion::Collision {
            existing: existing
                .payload
                .as_ref()
                .map_or_else(|| Value::Null.to_string(), std::string::ToString::to_string),
        });
    }
    let payload: Value = serde_json::from_str(&fact.payload).map_err(|e| {
        velesdb_core::Error::Query(format!("fact {} carries unreadable payload: {e}", fact.id))
    })?;
    any.upsert(vec![velesdb_core::Point::new(
        fact.id,
        vector.to_vec(),
        Some(payload),
    )])?;
    Ok(Reinsertion::Inserted)
}

/// What a batch re-insertion produced.
///
/// Aligned by id rather than by position, because the caller's question is
/// "which facts did NOT land", and an index into a slice it may have built by
/// filtering is not an answer it can act on.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BatchReinsertion {
    /// Facts written.
    pub inserted: u64,
    /// Ids already occupied. Nothing was written over any of them.
    pub collisions: Vec<u64>,
}

/// Put a whole batch back, in one write.
///
/// The reason a batch exists at all is throughput — a per-fact write costs an
/// fsync each, which is what made a rebuild of a real store look impossible
/// before #1797. The reason it is dangerous is that batching is exactly where
/// an id, a reserved key or an expiry gets dropped without anyone noticing,
/// since nothing fails.
///
/// Occupied ids are collected FIRST and excluded from the write, so a batch
/// containing one collision still lands the rest and still overwrites nothing.
///
/// # Errors
/// Returns [`crate::MemoryError`] if the collection is absent, a payload is
/// unreadable, or the write fails.
pub fn reinsert_batch(
    db: &velesdb_core::Database,
    collection: &str,
    batch: &[(RawFact, Vec<f32>)],
) -> Result<BatchReinsertion, crate::MemoryError> {
    let any = db.get_any_collection(collection).ok_or_else(|| {
        velesdb_core::Error::Query(format!("collection `{collection}` not found"))
    })?;
    let ids: Vec<u64> = batch.iter().map(|(fact, _)| fact.id).collect();
    let occupied: std::collections::HashSet<u64> = any
        .get(&ids)
        .into_iter()
        .flatten()
        .map(|point| point.id)
        .collect();

    let mut points = Vec::with_capacity(batch.len());
    for (fact, vector) in batch {
        if occupied.contains(&fact.id) {
            continue;
        }
        let payload: Value = serde_json::from_str(&fact.payload).map_err(|e| {
            velesdb_core::Error::Query(format!("fact {} carries unreadable payload: {e}", fact.id))
        })?;
        points.push(velesdb_core::Point::new(
            fact.id,
            vector.clone(),
            Some(payload),
        ));
    }
    let inserted = u64::try_from(points.len()).unwrap_or(u64::MAX);
    if !points.is_empty() {
        any.upsert(points)?;
    }
    let mut collisions: Vec<u64> = occupied.into_iter().collect();
    collisions.sort_unstable();
    Ok(BatchReinsertion {
        inserted,
        collisions,
    })
}
