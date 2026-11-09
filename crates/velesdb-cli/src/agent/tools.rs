//! Whitelisted tools the agent may call — every one resolves to a real
//! `VelesQL`/Database operation, and only these names dispatch.

use velesdb_core::{collection::graph::TraversalConfig, Database};

/// The whitelist, in help order. Anything outside it is refused.
pub const TOOL_NAMES: &[&str] = &[
    "list",
    "describe",
    "search",
    "hybrid",
    "explain",
    "aggregate",
    "graph",
];

/// One-line contract per tool, embedded in the LLM prompt and offline help.
pub const TOOL_DOCS: &str = "\
list()                                  -> all collection names
describe(<collection>)                  -> stats + row count
search(<collection> <field> <term>)     -> SELECT … WHERE field MATCH 'term'
hybrid(<collection> <field> <term>)     -> same query flagged hybrid;
                                          the offline agent has no embedder so
                                          it runs the MATCH branch and says so
explain(<collection> <field> <term>)    -> EXPLAIN of that same query
aggregate(<collection> <field>)         -> SELECT field, COUNT(*) GROUP BY
graph(<graph> <node_id>)                -> BFS traversal from the node";

/// Dispatch `tool(args)`. Returns the observation text; refusals are `Err`.
pub fn call(db: &Database, tool: &str, args: &str) -> Result<String, String> {
    let a: Vec<&str> = args.split_whitespace().collect();
    match tool {
        "list" => Ok(list(db)),
        "describe" => describe(db, a.first().copied().unwrap_or("")),
        "search" => query_tool(db, &a, false),
        "hybrid" => query_tool(db, &a, true),
        "explain" => explain(db, &a),
        "aggregate" => aggregate(db, &a),
        "graph" => graph(db, &a),
        _ => Err(format!(
            "unknown tool '{tool}' (whitelist: {})",
            TOOL_NAMES.join(", ")
        )),
    }
}

fn list(db: &Database) -> String {
    let names = db.list_collections();
    if names.is_empty() {
        "no collections".to_string()
    } else {
        format!("{} collection(s): {}", names.len(), names.join(", "))
    }
}

fn describe(db: &Database, name: &str) -> Result<String, String> {
    if name.is_empty() {
        return Err("describe needs a collection name".to_string());
    }
    if !db.list_collections().iter().any(|n| n == name) {
        return Err(format!("collection '{name}' not found"));
    }
    let kind = if db.get_graph_collection(name).is_some() {
        "graph"
    } else {
        "vector/metadata"
    };
    match db.get_collection_stats(name) {
        Ok(Some(s)) => Ok(format!("{name}: kind={kind} stats={s:?}")),
        Ok(None) => Ok(format!("{name}: kind={kind} (no stats file yet)")),
        Err(e) => Err(format!("describe {name}: {e}")),
    }
}

/// Text search. `hybrid` is documented honestly: without an embedder the
/// agent can only run the MATCH branch, and the observation says so.
fn query_tool(db: &Database, a: &[&str], hybrid: bool) -> Result<String, String> {
    if a.len() < 3 {
        return Err(format!(
            "{} needs <collection> <field> <term>",
            if hybrid { "hybrid" } else { "search" }
        ));
    }
    let (coll, field, term) = (a[0], a[1], a[2..].join(" "));
    let sql = format!(
        "SELECT * FROM {coll} WHERE {field} MATCH '{}' LIMIT 5",
        term.replace('\'', "''")
    );
    let note = if hybrid {
        "hybrid requested; no embedder — MATCH branch only. "
    } else {
        ""
    };
    run_sql(db, &sql).map(|r| format!("{note}{r}"))
}

fn explain(db: &Database, a: &[&str]) -> Result<String, String> {
    if a.len() < 3 {
        return Err("explain needs <collection> <field> <term>".to_string());
    }
    let (coll, field, term) = (a[0], a[1], a[2..].join(" "));
    let sql = format!(
        "SELECT * FROM {coll} WHERE {field} MATCH '{}' LIMIT 5",
        term.replace('\'', "''")
    );
    let parsed = velesdb_core::velesql::Parser::parse(&sql).map_err(|e| format!("parse: {e}"))?;
    let plan = db
        .explain_query(&parsed)
        .map_err(|e| format!("explain: {e}"))?;
    Ok(format!("{sql}\n{}", plan.to_tree()))
}

fn aggregate(db: &Database, a: &[&str]) -> Result<String, String> {
    if a.len() < 2 {
        return Err("aggregate needs <collection> <field>".to_string());
    }
    let sql = format!("SELECT {}, COUNT(*) FROM {} GROUP BY {}", a[1], a[0], a[1]);
    run_sql(db, &sql)
}

fn graph(db: &Database, a: &[&str]) -> Result<String, String> {
    let (name, node) = match (a.first(), a.get(1)) {
        (Some(n), Some(id)) => (*n, *id),
        _ => return Err("graph needs <graph> <node_id>".to_string()),
    };
    let node: u64 = node
        .parse()
        .map_err(|_| format!("graph node '{node}' is not a u64"))?;
    let col = db
        .get_graph_collection(name)
        .ok_or_else(|| format!("graph '{name}' not found"))?;
    let cfg = TraversalConfig {
        min_depth: 1,
        max_depth: 2,
        limit: 50,
        ..TraversalConfig::default()
    };
    let hits = col.traverse_bfs(node, &cfg);
    let desc: Vec<String> = hits
        .iter()
        .map(|r| format!("{}(d{})", r.target_id, r.depth))
        .collect();
    Ok(format!(
        "{name} BFS depth<=2 from {node}: {} node(s): {}",
        hits.len(),
        desc.join(", ")
    ))
}

/// Run a VelesQL statement and flatten rows for the transcript.
fn run_sql(db: &Database, sql: &str) -> Result<String, String> {
    let res = crate::repl_execute::execute_query(db, sql, None, None)
        .map_err(|e| format!("query failed: {e}"))?;
    if res.rows.is_empty() {
        return Ok(format!("{sql} -> 0 rows"));
    }
    let mut out = format!("{sql} -> {} row(s):", res.rows.len());
    for row in res.rows.iter().take(10) {
        out.push_str(&format!(
            "\n  {}",
            serde_json::to_string(row).unwrap_or_default()
        ));
    }
    if res.rows.len() > 10 {
        out.push_str(&format!("\n  … {} more", res.rows.len() - 10));
    }
    Ok(out)
}
