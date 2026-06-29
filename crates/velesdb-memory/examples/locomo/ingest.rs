//! Build the memory store from extracted facts.
//!
//! Each fact becomes a memory tagged with its source `dia_id`s. Each salient
//! entity becomes a hub memory, and every fact is `relate()`d to its entities
//! with an `about` edge. That fact↔entity graph is exactly what `why()` walks:
//! seed a fact by vector similarity, hop to its entities, reach sibling facts a
//! pure vector search would never surface.

use std::collections::{HashMap, HashSet};
use std::error::Error;

use serde_json::{Map, Value};
use velesdb_memory::DynEmbedder;
use velesdb_memory::MemoryService;

use crate::bm25::Bm25Index;
use crate::extract::Fact;

/// The ingested store: the service plus a fact-id → source-`dia_id`s index
/// (entity hubs are excluded from it, so retrieval can skip them), and the
/// fact↔entity structure that lets fusion weight a graph link by how *specific*
/// the connecting entity is (rare entity ⇒ strong signal; mega-hub ⇒ noise).
pub struct Store {
    pub svc: MemoryService<DynEmbedder>,
    facts: HashMap<u64, Vec<String>>,
    /// fact id → the entity-hub ids it is linked to.
    fact_entities: HashMap<u64, Vec<u64>>,
    /// entity-hub id → the fact ids that link to it (the inverse adjacency, for
    /// multi-seed graph reach without going through the single-seed `why()`).
    entity_facts: HashMap<u64, Vec<u64>>,
    /// fact id → its text, so a multi-seed graph reach can carry fact content
    /// into the context without a second `why()` round-trip.
    fact_text: HashMap<u64, String>,
    /// fact id → its session date (`YYYYMMDD`, 0 if unknown), so the generation
    /// context can be date-stamped — the dates are retrieved but otherwise never
    /// reach the answerer.
    fact_ts: HashMap<u64, i64>,
    /// entity-hub id → its degree (how many facts link to it).
    entity_degree: HashMap<u64, usize>,
    /// Total fact memories, for the inverse-document-frequency weighting.
    n_facts: usize,
    /// Lexical (BM25) index over fact texts, fused with dense recall via RRF.
    bm25: Bm25Index,
}

impl Store {
    /// True when `id` is a fact memory (not an entity hub).
    pub fn is_fact(&self, id: u64) -> bool {
        self.facts.contains_key(&id)
    }

    /// The source `dia_id`s of a fact, or empty for non-facts.
    pub fn dia_ids(&self, id: u64) -> &[String] {
        self.facts.get(&id).map_or(&[], Vec::as_slice)
    }

    /// The entity-hub ids a fact is linked to (empty for non-facts).
    pub fn fact_entity_ids(&self, id: u64) -> &[u64] {
        self.fact_entities.get(&id).map_or(&[], Vec::as_slice)
    }

    /// The fact ids linked to an entity hub (its sibling facts).
    pub fn entity_fact_ids(&self, eid: u64) -> &[u64] {
        self.entity_facts.get(&eid).map_or(&[], Vec::as_slice)
    }

    /// A fact's text (empty for non-facts).
    pub fn fact_text(&self, id: u64) -> &str {
        self.fact_text.get(&id).map_or("", String::as_str)
    }

    /// A fact's session date as `YYYYMMDD` (0 if unknown / non-fact).
    pub fn fact_ts(&self, id: u64) -> i64 {
        self.fact_ts.get(&id).copied().unwrap_or(0)
    }

    /// The latest session date across all facts — the conversation's "now", the
    /// reference point a temporal question ("how long ago", "how long after")
    /// reasons from.
    pub fn latest_ts(&self) -> i64 {
        self.fact_ts.values().copied().max().unwrap_or(0)
    }

    /// Every source `dia_id` that the extractor actually produced a fact for —
    /// the upper bound on what retrieval can ever surface. A gold evidence
    /// `dia_id` absent here is an *extraction* miss, not a retrieval miss.
    pub fn extracted_dia_ids(&self) -> HashSet<&str> {
        self.facts
            .values()
            .flat_map(|ids| ids.iter().map(String::as_str))
            .collect()
    }

    /// Fact ids ranked by BM25 lexical relevance to `query` (best first).
    pub fn bm25_search(&self, query: &str) -> Vec<u64> {
        self.bm25.search(query)
    }

    /// Normalised inverse-document-frequency of an entity hub, in `[0, 1]`: `1`
    /// when the entity is unique to a single fact (maximally specific), trending
    /// to `0` as it links ever more facts (a topical mega-hub whose connections
    /// carry little answer signal). This is the weight that lets the graph
    /// promote a rare, specific link over a generic one.
    pub fn entity_idf(&self, eid: u64) -> f64 {
        let degree = self.entity_degree.get(&eid).copied().unwrap_or(0);
        if degree == 0 || self.n_facts <= 1 {
            return 0.0;
        }
        let n = f64::from(u32::try_from(self.n_facts).unwrap_or(u32::MAX));
        let d = f64::from(u32::try_from(degree).unwrap_or(u32::MAX));
        (n / d).ln() / n.ln()
    }
}

/// Ingest `facts` into a fresh service, wiring the fact↔entity graph.
pub fn build(svc: MemoryService<DynEmbedder>, facts: &[Fact]) -> Result<Store, Box<dyn Error>> {
    let mut store = Store {
        svc,
        facts: HashMap::new(),
        fact_entities: HashMap::new(),
        entity_facts: HashMap::new(),
        fact_text: HashMap::new(),
        fact_ts: HashMap::new(),
        entity_degree: HashMap::new(),
        n_facts: 0,
        bm25: Bm25Index::build(std::iter::empty()),
    };
    let mut entity_ids: HashMap<String, u64> = HashMap::new();
    let mut edges: HashSet<(u64, u64)> = HashSet::new();

    for fact in facts {
        let fid = remember_fact(&store.svc, fact)?;
        record_fact(&mut store, fid, fact);
        let eids = wire_entities(&store.svc, &mut entity_ids, &mut edges, fid, &fact.entities)?;
        record_entities(&mut store, fid, &eids);
    }
    // Degree = number of distinct facts each entity links, derived from the
    // deduplicated fact→entity map so a recurring fact text never double-counts.
    let mut degree: HashMap<u64, usize> = HashMap::new();
    let mut entity_facts: HashMap<u64, Vec<u64>> = HashMap::new();
    for (&fid, eids) in &store.fact_entities {
        for &eid in eids {
            *degree.entry(eid).or_insert(0) += 1;
            entity_facts.entry(eid).or_default().push(fid);
        }
    }
    store.entity_degree = degree;
    store.entity_facts = entity_facts;
    store.n_facts = store.facts.len();
    store.bm25 = Bm25Index::build(
        store
            .fact_text
            .iter()
            .map(|(id, text)| (*id, text.as_str())),
    );
    report_density(facts, &entity_ids);
    Ok(store)
}

/// Record the (deduplicated) entity-hub ids a fact is linked to, merging when
/// the same fact text recurs.
fn record_entities(store: &mut Store, fid: u64, eids: &[u64]) {
    let entry = store.fact_entities.entry(fid).or_default();
    for &eid in eids {
        if !entry.contains(&eid) {
            entry.push(eid);
        }
    }
}

/// Print how connectable the graph is: only entities shared by ≥2 facts can
/// carry a multi-hop traversal, so a near-zero "shared" count explains a graph
/// that contributes nothing — the structure, not the query, is the limit.
fn report_density(facts: &[Fact], entity_ids: &HashMap<String, u64>) {
    let mut degree: HashMap<&str, u32> = HashMap::new();
    for fact in facts {
        for entity in &fact.entities {
            *degree.entry(entity.as_str()).or_insert(0) += 1;
        }
    }
    let shared = degree.values().filter(|&&d| d >= 2).count();
    eprintln!(
        "        graph: {} entities ({shared} shared by ≥2 facts)",
        entity_ids.len()
    );
}

/// Link a fact to each of its entities with a deduplicated edge in *both*
/// directions. `why()` only follows outgoing edges, so the fact→entity edge
/// alone leaves entity hubs as dead ends; the entity→fact edge is what lets a
/// traversal hop from one fact, through a shared entity, to its sibling facts —
/// the multi-hop path the benchmark measures.
fn wire_entities(
    svc: &MemoryService<DynEmbedder>,
    entity_ids: &mut HashMap<String, u64>,
    edges: &mut HashSet<(u64, u64)>,
    fid: u64,
    entities: &[String],
) -> Result<Vec<u64>, Box<dyn Error>> {
    let mut eids = Vec::with_capacity(entities.len());
    for entity in entities {
        let eid = entity_hub(svc, entity_ids, entity)?;
        eids.push(eid);
        if edges.insert((fid, eid)) {
            svc.relate(fid, eid, "about")?;
        }
        if edges.insert((eid, fid)) {
            svc.relate(eid, fid, "mentions")?;
        }
    }
    Ok(eids)
}

/// Remember one fact with its `dia_id`s in metadata; dedup is by fact text
/// (the service keys memories on a stable hash of the content).
fn remember_fact(svc: &MemoryService<DynEmbedder>, fact: &Fact) -> Result<u64, Box<dyn Error>> {
    let mut meta = Map::new();
    let ids: Vec<Value> = fact.dia_ids.iter().cloned().map(Value::String).collect();
    meta.insert("dia_ids".to_string(), Value::Array(ids));
    // The ColumnStore facet: the session date, range-queryable via recall_where.
    if fact.ts != 0 {
        meta.insert("ts".to_string(), Value::from(fact.ts));
    }
    Ok(svc.remember(&fact.text, &[], Some(&meta))?)
}

/// Index a fact id, merging `dia_id`s when the same fact text recurs.
fn record_fact(store: &mut Store, fid: u64, fact: &Fact) {
    let dia_ids = store.facts.entry(fid).or_default();
    for id in &fact.dia_ids {
        if !dia_ids.contains(id) {
            dia_ids.push(id.clone());
        }
    }
    store
        .fact_text
        .entry(fid)
        .or_insert_with(|| fact.text.clone());
    store.fact_ts.entry(fid).or_insert(fact.ts);
}

/// Get or create the hub memory for `entity`, caching its id.
fn entity_hub(
    svc: &MemoryService<DynEmbedder>,
    entity_ids: &mut HashMap<String, u64>,
    entity: &str,
) -> Result<u64, Box<dyn Error>> {
    if let Some(&id) = entity_ids.get(entity) {
        return Ok(id);
    }
    let mut meta = Map::new();
    meta.insert("kind".to_string(), Value::String("entity".to_string()));
    let id = svc.remember(&format!("Entity: {entity}"), &[], Some(&meta))?;
    entity_ids.insert(entity.to_string(), id);
    Ok(id)
}
