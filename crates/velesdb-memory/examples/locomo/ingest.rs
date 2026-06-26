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
use velesdb_memory::mcp::DynEmbedder;
use velesdb_memory::MemoryService;

use crate::extract::Fact;

/// The ingested store: the service plus a fact-id → source-`dia_id`s index
/// (entity hubs are excluded from it, so retrieval can skip them).
pub struct Store {
    pub svc: MemoryService<DynEmbedder>,
    facts: HashMap<u64, Vec<String>>,
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
}

/// Ingest `facts` into a fresh service, wiring the fact↔entity graph.
pub fn build(svc: MemoryService<DynEmbedder>, facts: &[Fact]) -> Result<Store, Box<dyn Error>> {
    let mut store = Store {
        svc,
        facts: HashMap::new(),
    };
    let mut entity_ids: HashMap<String, u64> = HashMap::new();
    let mut edges: HashSet<(u64, u64)> = HashSet::new();

    for fact in facts {
        let fid = remember_fact(&store.svc, fact)?;
        record_fact(&mut store, fid, fact);
        wire_entities(&store.svc, &mut entity_ids, &mut edges, fid, &fact.entities)?;
    }
    report_density(facts, &entity_ids);
    Ok(store)
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
) -> Result<(), Box<dyn Error>> {
    for entity in entities {
        let eid = entity_hub(svc, entity_ids, entity)?;
        if edges.insert((fid, eid)) {
            svc.relate(fid, eid, "about")?;
        }
        if edges.insert((eid, fid)) {
            svc.relate(eid, fid, "mentions")?;
        }
    }
    Ok(())
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
