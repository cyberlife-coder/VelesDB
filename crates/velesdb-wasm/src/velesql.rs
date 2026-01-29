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
            .map_err(|e| {
                JsValue::from_str(&format!(
                    "VelesQL syntax error at position {}: {} (near '{}')",
                    e.position, e.message, e.fragment
                ))
            })
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

    /// Get the table name from the FROM clause.
    #[wasm_bindgen(getter, js_name = tableName)]
    pub fn table_name(&self) -> Option<String> {
        let from = &self.inner.select.from;
        if from.is_empty() {
            None
        } else {
            Some(from.clone())
        }
    }

    /// Get the list of selected columns as JSON array.
    #[wasm_bindgen(getter)]
    pub fn columns(&self) -> JsValue {
        use velesdb_core::velesql::SelectColumns;
        let cols: Vec<String> = match &self.inner.select.columns {
            SelectColumns::All => vec!["*".to_string()],
            SelectColumns::Columns(cols) => cols.iter().map(|c| c.name.clone()).collect(),
            SelectColumns::Aggregations(aggs) => aggs
                .iter()
                .map(|a| format!("{:?}", a.function_type))
                .collect(),
            SelectColumns::Mixed {
                columns,
                aggregations,
            } => {
                let mut result: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
                result.extend(
                    aggregations
                        .iter()
                        .map(|a| format!("{:?}", a.function_type)),
                );
                result
            }
        };
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
            Self::condition_has_vector_search(cond)
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
        let order_by: Vec<(String, String)> = match &self.inner.select.order_by {
            Some(order_items) => order_items
                .iter()
                .map(|item| {
                    let dir = if item.descending { "DESC" } else { "ASC" };
                    let col = match &item.expr {
                        velesdb_core::velesql::OrderByExpr::Field(f) => f.clone(),
                        velesdb_core::velesql::OrderByExpr::Similarity(_) => {
                            "similarity()".to_string()
                        }
                        velesdb_core::velesql::OrderByExpr::Aggregate(agg) => {
                            format!("{:?}", agg.function_type)
                        }
                    };
                    (col, dir.to_string())
                })
                .collect(),
            None => Vec::new(),
        };
        serde_wasm_bindgen::to_value(&order_by).unwrap_or(JsValue::NULL)
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

impl ParsedQuery {
    /// Recursively check if a condition contains vector search.
    fn condition_has_vector_search(cond: &velesdb_core::velesql::Condition) -> bool {
        use velesdb_core::velesql::Condition;

        match cond {
            Condition::VectorSearch(_) | Condition::VectorFusedSearch { .. } => true,
            Condition::And(left, right) | Condition::Or(left, right) => {
                Self::condition_has_vector_search(left) || Self::condition_has_vector_search(right)
            }
            Condition::Group(inner) | Condition::Not(inner) => {
                Self::condition_has_vector_search(inner)
            }
            _ => false,
        }
    }
}

// Tests use velesdb_core::velesql::Parser directly to avoid wasm_bindgen issues in native tests
#[cfg(test)]
mod tests {
    use velesdb_core::velesql::Parser;

    // === MATCH Query Tests (EPIC-053 US-004) ===

    #[test]
    fn test_parse_match_query() {
        let parsed = Parser::parse("MATCH (p:Person)-[:KNOWS]->(f:Person) RETURN f.name");
        assert!(parsed.is_ok(), "MATCH query should parse: {parsed:?}");
        let query = parsed.unwrap();
        assert!(query.is_match_query());
        assert!(!query.is_select_query());
    }

    #[test]
    fn test_match_query_node_count() {
        let parsed = Parser::parse("MATCH (p:Person)-[:KNOWS]->(f:Person) RETURN f.name").unwrap();
        let mc = parsed
            .match_clause
            .as_ref()
            .expect("should have match_clause");
        assert_eq!(mc.patterns[0].nodes.len(), 2);
    }

    #[test]
    fn test_match_query_relationship_count() {
        let parsed = Parser::parse("MATCH (a)-[:REL1]->(b)-[:REL2]->(c) RETURN a, b, c").unwrap();
        let mc = parsed
            .match_clause
            .as_ref()
            .expect("should have match_clause");
        assert_eq!(mc.patterns[0].relationships.len(), 2);
    }

    #[test]
    fn test_match_query_node_labels() {
        let parsed = Parser::parse("MATCH (p:Person:Author) RETURN p").unwrap();
        let mc = parsed
            .match_clause
            .as_ref()
            .expect("should have match_clause");
        let node = &mc.patterns[0].nodes[0];
        assert!(node.labels.contains(&"Person".to_string()));
    }

    #[test]
    fn test_match_query_relationship_types() {
        let parsed = Parser::parse("MATCH (a)-[:WROTE]->(b) RETURN b").unwrap();
        let mc = parsed
            .match_clause
            .as_ref()
            .expect("should have match_clause");
        let rel = &mc.patterns[0].relationships[0];
        assert!(rel.types.contains(&"WROTE".to_string()));
    }

    #[test]
    fn test_match_query_without_where() {
        // MATCH queries without WHERE work correctly
        let parsed = Parser::parse("MATCH (p:Person) RETURN p.name").unwrap();
        let mc = parsed
            .match_clause
            .as_ref()
            .expect("should have match_clause");
        assert!(mc.where_clause.is_none());
    }

    #[test]
    fn test_match_query_return_items() {
        let parsed = Parser::parse("MATCH (p:Person) RETURN p.name, p.age AS years").unwrap();
        let mc = parsed
            .match_clause
            .as_ref()
            .expect("should have match_clause");
        assert_eq!(mc.return_clause.items.len(), 2);
        assert_eq!(mc.return_clause.items[1].alias, Some("years".to_string()));
    }

    #[test]
    fn test_match_query_limit() {
        let parsed = Parser::parse("MATCH (p:Person) RETURN p LIMIT 10").unwrap();
        let mc = parsed
            .match_clause
            .as_ref()
            .expect("should have match_clause");
        assert_eq!(mc.return_clause.limit, Some(10));
    }

    // === Original SELECT Tests ===

    #[test]
    fn test_parse_simple_select() {
        let parsed = Parser::parse("SELECT * FROM documents LIMIT 10");
        assert!(parsed.is_ok());
        let query = parsed.unwrap();
        assert!(query.is_select_query());
        assert!(!query.is_match_query());
        assert_eq!(query.select.from, "documents");
        assert_eq!(query.select.limit, Some(10));
    }

    #[test]
    fn test_parse_invalid_query() {
        let parsed = Parser::parse("SELEC * FROM docs");
        assert!(parsed.is_err());
    }

    #[test]
    fn test_is_valid() {
        assert!(Parser::parse("SELECT * FROM docs").is_ok());
        assert!(Parser::parse("SELECT id FROM docs WHERE x = 1").is_ok());
        assert!(Parser::parse("SELEC * FROM docs").is_err());
    }

    #[test]
    fn test_parse_with_where() {
        let parsed = Parser::parse("SELECT * FROM docs WHERE category = 'tech'").unwrap();
        assert!(parsed.select.where_clause.is_some());
    }

    #[test]
    fn test_parse_vector_search() {
        let parsed = Parser::parse("SELECT * FROM docs WHERE vector NEAR $v LIMIT 10").unwrap();
        assert!(parsed.select.where_clause.is_some());
    }

    #[test]
    fn test_parse_with_order_by() {
        let parsed = Parser::parse("SELECT * FROM docs ORDER BY created_at DESC").unwrap();
        assert!(parsed.select.order_by.is_some());
    }

    #[test]
    fn test_parse_with_join() {
        let parsed =
            Parser::parse("SELECT * FROM orders JOIN products ON orders.product_id = products.id")
                .unwrap();
        assert!(!parsed.select.joins.is_empty());
        assert_eq!(parsed.select.joins.len(), 1);
    }

    #[test]
    fn test_parse_with_group_by() {
        let parsed =
            Parser::parse("SELECT category, COUNT(*) FROM products GROUP BY category").unwrap();
        assert!(parsed.select.group_by.is_some());
    }
}
