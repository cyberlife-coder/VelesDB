//! `VelesQL` query support for WASM (EPIC-056 US-004/005/006).
//!
//! Provides `VelesQL` parsing and validation for browser-based queries.

use wasm_bindgen::prelude::*;

/// `VelesQL` query parser for browser use.
///
/// # Example (JavaScript)
///
/// ```javascript
/// import { VelesQL } from 'velesdb-wasm';
///
/// // Parse a query
/// const parsed = VelesQL.parse("SELECT * FROM docs WHERE category = 'tech' LIMIT 10");
/// console.log(parsed.tableName);  // "docs"
/// console.log(parsed.isValid);    // true
///
/// // Validate without parsing
/// const valid = VelesQL.isValid("SELECT * FROM docs");  // true
/// ```
#[wasm_bindgen]
pub struct VelesQL;

#[wasm_bindgen]
impl VelesQL {
    /// Parse a `VelesQL` query string.
    ///
    /// Returns a `ParsedQuery` object with query introspection methods.
    /// Throws an error if the query has syntax errors.
    #[wasm_bindgen]
    pub fn parse(query: &str) -> Result<ParsedQuery, JsValue> {
        velesdb_core::velesql::Parser::parse(query)
            .map(|q| ParsedQuery { inner: q })
            .map_err(|e| crate::wasm_error::WasmError::from(e).into_js_value())
    }

    /// Validate a `VelesQL` query without full parsing.
    ///
    /// This is faster than `parse()` when you only need to check validity.
    #[wasm_bindgen(js_name = isValid)]
    pub fn is_valid(query: &str) -> bool {
        velesdb_core::velesql::Parser::parse(query).is_ok()
    }
}

/// A parsed `VelesQL` statement with introspection methods.
#[wasm_bindgen]
pub struct ParsedQuery {
    inner: velesdb_core::velesql::Query,
}

#[wasm_bindgen]
impl ParsedQuery {
    /// Check if the query is valid (always true for successfully parsed queries).
    #[wasm_bindgen(getter, js_name = isValid)]
    pub fn is_valid(&self) -> bool {
        true
    }

    /// Check if this is a SELECT query.
    #[wasm_bindgen(getter, js_name = isSelect)]
    pub fn is_select(&self) -> bool {
        self.inner.is_select_query()
    }

    /// Check if this is a MATCH (graph) query.
    #[wasm_bindgen(getter, js_name = isMatch)]
    pub fn is_match(&self) -> bool {
        self.inner.is_match_query()
    }

    /// Get the collection name from the FROM clause, DDL, or DML statement.
    ///
    /// For DDL statements, returns the collection name from the DDL AST.
    /// For DML statements, returns the collection from the DML struct.
    /// Alias: `tableName` is kept for backward compatibility.
    #[wasm_bindgen(getter, js_name = collectionName)]
    pub fn collection_name(&self) -> Option<String> {
        // DDL: collection name is in the DDL AST, not in SELECT FROM.
        if let Some(ddl) = &self.inner.ddl {
            return Some(match ddl {
                velesdb_core::velesql::DdlStatement::CreateCollection(s) => s.name.clone(),
                velesdb_core::velesql::DdlStatement::DropCollection(s) => s.name.clone(),
                velesdb_core::velesql::DdlStatement::CreateIndex(s) => s.collection.clone(),
                velesdb_core::velesql::DdlStatement::DropIndex(s) => s.collection.clone(),
                velesdb_core::velesql::DdlStatement::Analyze(s) => s.collection.clone(),
                velesdb_core::velesql::DdlStatement::Truncate(s) => s.collection.clone(),
                velesdb_core::velesql::DdlStatement::AlterCollection(s) => s.collection.clone(),
                _ => return None,
            });
        }
        // DML: collection name is in the DML struct, not in SELECT FROM.
        if let Some(name) = crate::velesql_helpers::dml_collection_name(&self.inner) {
            return Some(name);
        }
        let from = &self.inner.select.from;
        if from.is_empty() {
            None
        } else {
            Some(from.clone())
        }
    }

    /// Legacy alias for `collectionName`. Prefer `collectionName`.
    #[wasm_bindgen(getter, js_name = tableName)]
    pub fn table_name(&self) -> Option<String> {
        self.collection_name()
    }

    /// Get the list of selected columns — one entry per SELECT-list item.
    ///
    /// Returns a JSON array with every item in the SELECT list, in grammar
    /// order: regular columns, aggregate calls, `similarity()` expressions,
    /// qualified wildcards (`alias.*`), and window functions. `SELECT *`
    /// returns `["*"]`.
    ///
    /// Note (v1.13.0 contract completion): versions prior to v1.13.0
    /// silently omitted `similarity()` expressions and qualified wildcards
    /// from this list for mixed SELECT statements. The full list is now
    /// returned. Callers that hard-coded the shorter length must update.
    #[wasm_bindgen(getter)]
    pub fn columns(&self) -> JsValue {
        let cols = self.inner.select.columns.to_display_names();
        serde_wasm_bindgen::to_value(&cols).unwrap_or(JsValue::NULL)
    }

    /// Check if DISTINCT modifier is present.
    #[wasm_bindgen(getter, js_name = hasDistinct)]
    pub fn has_distinct(&self) -> bool {
        !matches!(
            self.inner.select.distinct,
            velesdb_core::velesql::DistinctMode::None
        )
    }

    /// Check if the query has a WHERE clause.
    #[wasm_bindgen(getter, js_name = hasWhereClause)]
    pub fn has_where_clause(&self) -> bool {
        self.inner.select.where_clause.is_some()
    }

    /// Check if the query has an ORDER BY clause.
    #[wasm_bindgen(getter, js_name = hasOrderBy)]
    pub fn has_order_by(&self) -> bool {
        self.inner.select.order_by.is_some()
    }

    /// Check if the query has a GROUP BY clause.
    #[wasm_bindgen(getter, js_name = hasGroupBy)]
    pub fn has_group_by(&self) -> bool {
        self.inner.select.group_by.is_some()
    }

    /// Check if the query has JOINs.
    #[wasm_bindgen(getter, js_name = hasJoins)]
    pub fn has_joins(&self) -> bool {
        !self.inner.select.joins.is_empty()
    }

    /// Check if the query uses FUSION (hybrid search).
    #[wasm_bindgen(getter, js_name = hasFusion)]
    pub fn has_fusion(&self) -> bool {
        self.inner.select.fusion_clause.is_some()
    }

    /// Check if the query contains vector search (NEAR clause).
    #[wasm_bindgen(getter, js_name = hasVectorSearch)]
    pub fn has_vector_search(&self) -> bool {
        if let Some(ref cond) = self.inner.select.where_clause {
            crate::velesql_helpers::condition_has_vector_search(cond)
        } else {
            false
        }
    }

    /// Get the LIMIT value if present.
    #[wasm_bindgen(getter)]
    pub fn limit(&self) -> Option<u64> {
        self.inner.select.limit
    }

    /// Get the OFFSET value if present.
    #[wasm_bindgen(getter)]
    pub fn offset(&self) -> Option<u64> {
        self.inner.select.offset
    }

    /// Get the ORDER BY columns and directions as JSON array.
    #[wasm_bindgen(getter, js_name = orderBy)]
    pub fn order_by(&self) -> JsValue {
        let pairs: Vec<(String, String)> = self
            .inner
            .select
            .order_by
            .as_deref()
            .map_or_else(Vec::new, |items| {
                items.iter().map(|item| item.to_display_pair()).collect()
            });
        serde_wasm_bindgen::to_value(&pairs).unwrap_or(JsValue::NULL)
    }

    /// Get the GROUP BY columns as JSON array.
    #[wasm_bindgen(getter, js_name = groupBy)]
    pub fn group_by(&self) -> JsValue {
        let group_by: Vec<String> = match &self.inner.select.group_by {
            Some(gb) => gb.columns.clone(),
            None => Vec::new(),
        };
        serde_wasm_bindgen::to_value(&group_by).unwrap_or(JsValue::NULL)
    }

    /// Get the number of JOIN clauses.
    #[wasm_bindgen(getter, js_name = joinCount)]
    pub fn join_count(&self) -> usize {
        self.inner.select.joins.len()
    }

    // === DDL/DML Introspection (VelesQL v3.3) ===

    /// Check if this is a DDL query (CREATE/DROP COLLECTION).
    #[wasm_bindgen(getter, js_name = isDdl)]
    pub fn is_ddl(&self) -> bool {
        self.inner.is_ddl_query()
    }

    /// Check if this is a DML mutation (INSERT/UPDATE/DELETE/INSERT EDGE/DELETE EDGE).
    #[wasm_bindgen(getter, js_name = isDml)]
    pub fn is_dml(&self) -> bool {
        self.inner.is_dml_query()
    }

    /// Check if this is a DELETE statement (DELETE FROM or DELETE EDGE).
    #[wasm_bindgen(getter, js_name = isDelete)]
    pub fn is_delete(&self) -> bool {
        matches!(
            &self.inner.dml,
            Some(velesdb_core::velesql::DmlStatement::Delete(_))
                | Some(velesdb_core::velesql::DmlStatement::DeleteEdge(_))
        )
    }

    /// Check if this is an INSERT EDGE statement.
    #[wasm_bindgen(getter, js_name = isInsertEdge)]
    pub fn is_insert_edge(&self) -> bool {
        matches!(
            &self.inner.dml,
            Some(velesdb_core::velesql::DmlStatement::InsertEdge(_))
        )
    }

    // === MATCH Query Introspection (EPIC-053 US-004) ===

    /// Get the number of node patterns in the MATCH clause.
    #[wasm_bindgen(getter, js_name = matchNodeCount)]
    pub fn match_node_count(&self) -> usize {
        self.inner
            .match_clause
            .as_ref()
            .map_or(0, |mc| mc.patterns.first().map_or(0, |p| p.nodes.len()))
    }

    /// Get the number of relationship patterns in the MATCH clause.
    #[wasm_bindgen(getter, js_name = matchRelationshipCount)]
    pub fn match_relationship_count(&self) -> usize {
        self.inner.match_clause.as_ref().map_or(0, |mc| {
            mc.patterns.first().map_or(0, |p| p.relationships.len())
        })
    }

    /// Get node labels from the MATCH clause as JSON array of arrays.
    /// Each inner array contains the labels for one node pattern.
    #[wasm_bindgen(getter, js_name = matchNodeLabels)]
    pub fn match_node_labels(&self) -> JsValue {
        let labels: Vec<Vec<String>> = self
            .inner
            .match_clause
            .as_ref()
            .map(|mc| {
                mc.patterns
                    .first()
                    .map(|p| p.nodes.iter().map(|n| n.labels.clone()).collect())
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        serde_wasm_bindgen::to_value(&labels).unwrap_or(JsValue::NULL)
    }

    /// Get relationship types from the MATCH clause as JSON array of arrays.
    /// Each inner array contains the types for one relationship pattern.
    #[wasm_bindgen(getter, js_name = matchRelationshipTypes)]
    pub fn match_relationship_types(&self) -> JsValue {
        let types: Vec<Vec<String>> = self
            .inner
            .match_clause
            .as_ref()
            .map(|mc| {
                mc.patterns
                    .first()
                    .map(|p| p.relationships.iter().map(|r| r.types.clone()).collect())
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        serde_wasm_bindgen::to_value(&types).unwrap_or(JsValue::NULL)
    }

    /// Get RETURN items from the MATCH clause as JSON array.
    #[wasm_bindgen(getter, js_name = matchReturnItems)]
    pub fn match_return_items(&self) -> JsValue {
        let items: Vec<(String, Option<String>)> = self
            .inner
            .match_clause
            .as_ref()
            .map(|mc| {
                mc.return_clause
                    .items
                    .iter()
                    .map(|i| (i.expression.clone(), i.alias.clone()))
                    .collect()
            })
            .unwrap_or_default();
        serde_wasm_bindgen::to_value(&items).unwrap_or(JsValue::NULL)
    }

    /// Get the LIMIT from the MATCH RETURN clause.
    #[wasm_bindgen(getter, js_name = matchLimit)]
    pub fn match_limit(&self) -> Option<u64> {
        self.inner
            .match_clause
            .as_ref()
            .and_then(|mc| mc.return_clause.limit)
    }

    /// Check if the MATCH clause has a WHERE condition.
    #[wasm_bindgen(getter, js_name = matchHasWhere)]
    pub fn match_has_where(&self) -> bool {
        self.inner
            .match_clause
            .as_ref()
            .is_some_and(|mc| mc.where_clause.is_some())
    }
}

// Tests use velesdb_core::velesql::Parser directly to avoid wasm_bindgen issues in native tests
#[cfg(test)]
#[path = "velesql_tests.rs"]
mod tests;
