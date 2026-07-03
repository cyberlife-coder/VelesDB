//! In-memory [`MemoryStore`] backend for the browser: no filesystem, no
//! `persistence` feature, no network. This is what lets `velesdb-memory`'s
//! agent-memory wedge (`remember`/`recall`/`recall_fused`/`relate`/`forget`/
//! `why`) run entirely client-side.
//!
//! Not durable across a page reload by itself — pair with this crate's own
//! `IndexedDB` persistence (`vector_store_persistence.rs`) if that's needed;
//! this store only holds state in the WASM heap.
//!
//! Reuses the crate's existing similarity math ([`vector_ops::compute_scores`])
//! and edge storage ([`WasmGraphStore`]) rather than duplicating them — this
//! module is the glue that makes those primitives satisfy `MemoryStore`, not
//! a second implementation of either.

use std::cell::RefCell;
use std::collections::HashMap;

use serde_json::{Map, Value};
use velesdb_memory::{
    ColumnFilter, ColumnOp, MemoryEdge, MemoryError, MemoryStore, Metadata, Recollection,
};

use crate::graph_store::WasmGraphStore;
use crate::{vector_ops, DistanceMetric, StorageMode};

/// Reserved payload key for a fact's durable expiry (a millisecond Unix
/// timestamp), mirroring `velesdb-core`'s `_veles_expires_at` naming so the
/// concept is consistent for anyone who knows the native store — the raw
/// units are a private implementation detail, since nothing outside this
/// module ever reads it directly (`get_metadata`'s reserved-key filtering
/// happens one layer up, in `velesdb-memory`'s `service.rs`).
const EXPIRES_AT_KEY: &str = "_veles_expires_at";

/// Current wall-clock time in milliseconds since the Unix epoch. Real time on
/// both targets — `wasm32` via `js_sys::Date`, native via `SystemTime` — so
/// TTL expiry is exercised by ordinary `cargo test`, not just in a browser.
#[cfg(target_arch = "wasm32")]
fn now_ms() -> u64 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let ms = js_sys::Date::now() as u64;
    ms
}

#[cfg(not(target_arch = "wasm32"))]
fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// One stored fact: its embedding and its full JSON payload (`content` plus
/// any caller metadata and the reserved expiry key).
struct Fact {
    embedding: Vec<f32>,
    payload: Map<String, Value>,
}

impl Fact {
    fn is_expired(&self) -> bool {
        self.payload
            .get(EXPIRES_AT_KEY)
            .and_then(Value::as_u64)
            .is_some_and(|expires_at| now_ms() >= expires_at)
    }
}

/// Build a fact payload: `content` plus every key of `metadata` (already
/// validated reserved-key-free by `velesdb-memory`'s orchestration layer),
/// plus a durable expiry if `ttl_seconds` is set.
fn build_payload(
    content: &str,
    metadata: Option<&Metadata>,
    ttl_seconds: Option<u64>,
) -> Map<String, Value> {
    let mut payload = metadata.cloned().unwrap_or_default();
    payload.insert("content".to_string(), Value::String(content.to_string()));
    if let Some(ttl) = ttl_seconds {
        payload.insert(
            EXPIRES_AT_KEY.to_string(),
            Value::from(now_ms().saturating_add(ttl.saturating_mul(1000))),
        );
    }
    payload
}

fn content_of(payload: &Map<String, Value>) -> String {
    payload
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// True when every key in `filter` is present in `payload` with an equal value.
fn matches_all(payload: &Map<String, Value>, filter: &Metadata) -> bool {
    filter.iter().all(|(k, v)| payload.get(k) == Some(v))
}

struct Inner {
    dimension: usize,
    facts: HashMap<u64, Fact>,
    /// Insertion order, so query results over a tiny corpus are at least
    /// deterministic when scores tie (no correctness dependency on this).
    order: Vec<u64>,
    graph: WasmGraphStore,
}

impl Inner {
    fn live_fact(&self, id: u64) -> Option<&Fact> {
        let fact = self.facts.get(&id)?;
        (!fact.is_expired()).then_some(fact)
    }
}

/// The in-memory [`MemoryStore`] backend `velesdb-wasm` supplies to
/// `MemoryService`, so the agent-memory wedge runs entirely in the browser.
pub struct WasmStore {
    inner: RefCell<Inner>,
}

impl WasmStore {
    /// Create an empty store sized for `dimension`-dimensional embeddings.
    #[must_use]
    pub fn new(dimension: usize) -> Self {
        Self {
            inner: RefCell::new(Inner {
                dimension,
                facts: HashMap::new(),
                order: Vec::new(),
                graph: WasmGraphStore::new(),
            }),
        }
    }

    fn put(&self, id: u64, embedding: &[f32], payload: Map<String, Value>) {
        let mut inner = self.inner.borrow_mut();
        if !inner.facts.contains_key(&id) {
            inner.order.push(id);
        }
        inner.facts.insert(
            id,
            Fact {
                embedding: embedding.to_vec(),
                payload,
            },
        );
    }
}

impl MemoryStore for WasmStore {
    fn store(&self, id: u64, content: &str, embedding: &[f32]) -> Result<(), MemoryError> {
        self.put(id, embedding, build_payload(content, None, None));
        Ok(())
    }

    fn store_with_metadata(
        &self,
        id: u64,
        content: &str,
        embedding: &[f32],
        metadata: &Metadata,
    ) -> Result<(), MemoryError> {
        self.put(id, embedding, build_payload(content, Some(metadata), None));
        Ok(())
    }

    fn store_with_ttl(
        &self,
        id: u64,
        content: &str,
        embedding: &[f32],
        ttl_seconds: u64,
    ) -> Result<(), MemoryError> {
        self.put(
            id,
            embedding,
            build_payload(content, None, Some(ttl_seconds)),
        );
        Ok(())
    }

    fn update_metadata(&self, id: u64, metadata: &Metadata) -> Result<(), MemoryError> {
        let mut inner = self.inner.borrow_mut();
        let Some(fact) = inner.facts.get_mut(&id) else {
            return Err(MemoryError::UnknownMemory(id));
        };
        for (key, value) in metadata {
            fact.payload.insert(key.clone(), value.clone());
        }
        Ok(())
    }

    fn get(&self, id: u64) -> Result<Option<(String, Vec<f32>)>, MemoryError> {
        let inner = self.inner.borrow();
        Ok(inner
            .live_fact(id)
            .map(|fact| (content_of(&fact.payload), fact.embedding.clone())))
    }

    fn get_metadata(&self, id: u64) -> Result<Option<Metadata>, MemoryError> {
        let inner = self.inner.borrow();
        Ok(inner.live_fact(id).map(|fact| fact.payload.clone()))
    }

    fn get_metadata_batch(&self, ids: &[u64]) -> Result<Vec<Option<Metadata>>, MemoryError> {
        let inner = self.inner.borrow();
        Ok(ids
            .iter()
            .map(|&id| inner.live_fact(id).map(|fact| fact.payload.clone()))
            .collect())
    }

    fn delete(&self, id: u64) -> Result<(), MemoryError> {
        let mut inner = self.inner.borrow_mut();
        inner.facts.remove(&id);
        inner.order.retain(|&x| x != id);
        // Cascade: an edge dangling off a deleted memory must not survive it
        // (matches the native store's cascading delete).
        inner
            .graph
            .delete_edges_where(|e| e.source == id || e.target == id);
        Ok(())
    }

    fn query_filtered(
        &self,
        embedding: &[f32],
        k: usize,
        filter: &Metadata,
        offset: usize,
    ) -> Result<Vec<(u64, f32, String)>, MemoryError> {
        self.query_scored(embedding, k, offset, |payload| matches_all(payload, filter))
    }

    fn query_excluding(
        &self,
        embedding: &[f32],
        k: usize,
        exclude: &Metadata,
    ) -> Result<Vec<(u64, f32, String)>, MemoryError> {
        self.query_scored(embedding, k, 0, |payload| {
            exclude.is_empty() || !matches_all(payload, exclude)
        })
    }

    fn query_columnar(
        &self,
        embedding: &[f32],
        k: usize,
        filters: &[ColumnFilter],
    ) -> Result<Vec<Recollection>, MemoryError> {
        for filter in filters {
            velesdb_memory::storage::validate_column_filter(filter)?;
        }
        self.query_ranked(
            embedding,
            k,
            0,
            |payload| columnar_matches(payload, filters),
            |id, score, fact| Recollection {
                id,
                score,
                content: content_of(&fact.payload),
                // Reserved keys (`content`, `_veles_*`) are stripped exactly
                // like the native backend and every other recall path — raw
                // payloads must never reach a caller-facing `Recollection`.
                metadata: velesdb_memory::storage::strip_reserved_keys_ref(Some(&fact.payload)),
            },
        )
    }

    fn relate(&self, from: u64, to: u64, relation: &str) -> Result<u64, MemoryError> {
        let mut inner = self.inner.borrow_mut();
        // The trait contract (and the native backend) reject an edge to a
        // missing or expired endpoint — a dangling edge would silently
        // inflate `entity_idf` degree counts and `why()` traversal work.
        for endpoint in [from, to] {
            if inner.live_fact(endpoint).is_none() {
                return Err(MemoryError::UnknownMemory(endpoint));
            }
        }
        // `explicit_id: None` derives the id from (source, target, label), so
        // the only documented failure mode (an explicit id collision) cannot
        // occur here — still mapped, not unwrapped, so a future signature
        // change can't silently reintroduce a panic path.
        inner
            .graph
            .insert_edge(None, from, to, relation.to_string(), None)
            .map_err(MemoryError::InvalidRelation)
    }

    fn relations(&self, id: u64) -> Result<Vec<MemoryEdge>, MemoryError> {
        let inner = self.inner.borrow();
        Ok(inner
            .graph
            .edges()
            .iter()
            // An edge into a TTL-expired fact is dead: the native backend
            // filters expired targets out, and `entity_idf` divides by this
            // degree — counting dead edges would under-weight every
            // graph-reached fact relative to the native ranking.
            .filter(|e| e.source == id && inner.live_fact(e.target).is_some())
            .map(|e| MemoryEdge {
                from: e.source,
                to: e.target,
                relation: e.label.clone(),
            })
            .collect())
    }

    fn count(&self) -> usize {
        let inner = self.inner.borrow();
        inner
            .order
            .iter()
            .filter(|&&id| inner.live_fact(id).is_some())
            .count()
    }
}

impl WasmStore {
    /// Shared vector-search core for [`MemoryStore::query_filtered`],
    /// [`MemoryStore::query_excluding`], and [`MemoryStore::query_columnar`]:
    /// score every non-expired fact whose payload passes `predicate` against
    /// `embedding`, rank, take `k` after `offset`, and build each returned
    /// row with `row`.
    ///
    /// `predicate` is applied while *building* the scoring input, borrowing
    /// each payload in place — nothing is cloned for a non-matching fact,
    /// and `row` runs only for the rows actually returned. Admitted facts
    /// are held as borrows (`matched`) and `row` receives the fact captured
    /// at scoring time — never re-fetched through the time-sensitive
    /// `live_fact` after ranking, so a TTL lapsing mid-query can't corrupt
    /// an admitted hit's content or metadata.
    fn query_ranked<T>(
        &self,
        embedding: &[f32],
        k: usize,
        offset: usize,
        predicate: impl Fn(&Map<String, Value>) -> bool,
        row: impl Fn(u64, f32, &Fact) -> T,
    ) -> Result<Vec<T>, MemoryError> {
        let inner = self.inner.borrow();
        let mut ids = Vec::new();
        let mut data = Vec::new();
        let mut matched: HashMap<u64, &Fact> = HashMap::new();
        for &id in &inner.order {
            let Some(fact) = inner.live_fact(id) else {
                continue;
            };
            if !predicate(&fact.payload) {
                continue;
            }
            ids.push(id);
            data.extend_from_slice(&fact.embedding);
            matched.insert(id, fact);
        }
        let mut scored = vector_ops::compute_scores(
            embedding,
            &ids,
            &data,
            &[],
            &[],
            &[],
            &[],
            inner.dimension,
            DistanceMetric::Cosine,
            StorageMode::Full,
        );
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        Ok(scored
            .into_iter()
            .skip(offset)
            .take(k)
            .filter_map(|(id, score)| matched.get(&id).map(|fact| row(id, score, fact)))
            .collect())
    }

    /// [`Self::query_ranked`] specialised to the `(id, score, content)`
    /// triple [`MemoryStore::query_filtered`]/[`MemoryStore::query_excluding`]
    /// return.
    fn query_scored(
        &self,
        embedding: &[f32],
        k: usize,
        offset: usize,
        predicate: impl Fn(&Map<String, Value>) -> bool,
    ) -> Result<Vec<(u64, f32, String)>, MemoryError> {
        self.query_ranked(embedding, k, offset, predicate, |id, score, fact| {
            (id, score, content_of(&fact.payload))
        })
    }
}

/// True when every filter in `filters` is satisfied by `payload` (AND-combined).
fn columnar_matches(payload: &Map<String, Value>, filters: &[ColumnFilter]) -> bool {
    filters.iter().all(|filter| {
        payload
            .get(&filter.field)
            .is_some_and(|value| compare(value, filter.op, &filter.value))
    })
}

/// Evaluate one [`ColumnOp`] between a stored payload value and the filter's
/// value: numeric comparison when both are numbers, lexicographic when both
/// are strings, equality-only otherwise (matches `VelesQL`'s scalar-comparison
/// semantics for the types `MemoryService::recall_where` accepts).
fn compare(stored: &Value, op: ColumnOp, target: &Value) -> bool {
    if let (Some(a), Some(b)) = (stored.as_f64(), target.as_f64()) {
        return match op {
            ColumnOp::Eq => (a - b).abs() < f64::EPSILON,
            ColumnOp::Ne => (a - b).abs() >= f64::EPSILON,
            ColumnOp::Lt => a < b,
            ColumnOp::Le => a <= b,
            ColumnOp::Gt => a > b,
            ColumnOp::Ge => a >= b,
        };
    }
    if let (Some(a), Some(b)) = (stored.as_str(), target.as_str()) {
        return match op {
            ColumnOp::Eq => a == b,
            ColumnOp::Ne => a != b,
            ColumnOp::Lt => a < b,
            ColumnOp::Le => a <= b,
            ColumnOp::Gt => a > b,
            ColumnOp::Ge => a >= b,
        };
    }
    match op {
        ColumnOp::Eq => stored == target,
        ColumnOp::Ne => stored != target,
        _ => false,
    }
}

#[cfg(test)]
#[path = "memory_store_tests.rs"]
mod tests;
