//! Complex VelesQL parser tests (EPIC-017 US-003/US-006).
//!
//! Tests combining: aggregates, multicolumn, vector search, graph patterns, EXPLAIN.
//! Based on research: VLDB 2024 hybrid vector+graph queries, cost estimation patterns.
#![cfg(all(test, feature = "persistence"))]

use crate::velesql::{
    AggregateArg, AggregateType, CompareOp, Condition, OrderByExpr, Parser, QueryPlan,
    SelectColumns, Value,
};

// =============================================================================
// CATEGORY 1: Pure Aggregation Queries
// =============================================================================

#[test]
fn test_parse_count_star_simple() {
    let query = Parser::parse("SELECT COUNT(*) FROM products").unwrap();
    match &query.select.columns {
        SelectColumns::Aggregations(aggs) => {
            assert_eq!(aggs.len(), 1);
            assert!(matches!(
                aggs[0].function_type,
                crate::velesql::AggregateType::Count
            ));
        }
        _ => panic!("Expected aggregations"),
    }
}

#[test]
fn test_parse_multiple_aggregates() {
    let query = Parser::parse(
        "SELECT COUNT(*), SUM(price), AVG(rating), MIN(stock), MAX(price) FROM products",
    )
    .unwrap();

    match &query.select.columns {
        SelectColumns::Aggregations(aggs) => {
            assert_eq!(aggs.len(), 5);
        }
        _ => panic!("Expected aggregations"),
    }
}

#[test]
fn test_parse_aggregate_with_alias() {
    let query =
        Parser::parse("SELECT COUNT(*) AS total, AVG(price) AS avg_price FROM products").unwrap();

    match &query.select.columns {
        SelectColumns::Aggregations(aggs) => {
            assert_eq!(aggs.len(), 2);
            assert_eq!(aggs[0].alias, Some("total".to_string()));
            assert_eq!(aggs[1].alias, Some("avg_price".to_string()));
        }
        _ => panic!("Expected aggregations"),
    }
}

// =============================================================================
// CATEGORY 2: GROUP BY + Aggregations
// =============================================================================

#[test]
fn test_parse_groupby_single_column() {
    let query = Parser::parse("SELECT category, COUNT(*) FROM products GROUP BY category").unwrap();

    assert!(query.select.group_by.is_some());
    let group_by = query.select.group_by.as_ref().unwrap();
    assert_eq!(group_by.columns, vec!["category"]);
}

#[test]
fn test_parse_groupby_multiple_columns() {
    let query = Parser::parse(
        "SELECT category, brand, COUNT(*), AVG(price) FROM products GROUP BY category, brand",
    )
    .unwrap();

    assert!(query.select.group_by.is_some());
    let group_by = query.select.group_by.as_ref().unwrap();
    assert_eq!(group_by.columns, vec!["category", "brand"]);
}

#[test]
fn test_parse_groupby_with_where() {
    let query = Parser::parse(
        "SELECT category, SUM(sales) FROM products WHERE active = true GROUP BY category",
    )
    .unwrap();

    assert!(query.select.where_clause.is_some());
    assert!(query.select.group_by.is_some());
}

// =============================================================================
// CATEGORY 3: HAVING Clause (Post-Aggregation Filters)
// =============================================================================

#[test]
fn test_parse_having_count() {
    let query = Parser::parse(
        "SELECT category, COUNT(*) FROM products GROUP BY category HAVING COUNT(*) > 10",
    )
    .unwrap();

    assert!(query.select.having.is_some());
    let having = query.select.having.as_ref().unwrap();
    assert!(!having.conditions.is_empty());
}

#[test]
fn test_parse_having_avg() {
    let query = Parser::parse(
        "SELECT brand, AVG(price) FROM products GROUP BY brand HAVING AVG(price) > 100",
    )
    .unwrap();

    assert!(query.select.having.is_some());
    let having = query.select.having.as_ref().unwrap();
    assert_eq!(having.conditions.len(), 1);
    assert_eq!(
        having.conditions[0].aggregate.function_type,
        AggregateType::Avg
    );
    assert_eq!(
        having.conditions[0].aggregate.argument,
        AggregateArg::Column("price".to_string())
    );
    assert_eq!(having.conditions[0].operator, CompareOp::Gt);
    assert_eq!(having.conditions[0].value, Value::Integer(100));
}

#[test]
fn test_parse_having_multiple_conditions() {
    let query = Parser::parse(
        "SELECT category, COUNT(*), AVG(price) FROM products \
         GROUP BY category \
         HAVING COUNT(*) > 5 AND AVG(price) < 500",
    )
    .unwrap();

    assert!(query.select.having.is_some());
    let having = query.select.having.as_ref().unwrap();
    assert_eq!(having.conditions.len(), 2);
}

// =============================================================================
// CATEGORY 4: Vector Search Queries
// =============================================================================

#[test]
fn test_parse_vector_near_basic() {
    let query =
        Parser::parse("SELECT * FROM embeddings WHERE vector NEAR $query LIMIT 10").unwrap();

    assert!(query.select.where_clause.is_some());
    assert_eq!(query.select.limit, Some(10));
}

#[test]
fn test_parse_vector_near_with_filter() {
    let query = Parser::parse(
        "SELECT * FROM embeddings WHERE vector NEAR $query AND category = 'tech' LIMIT 20",
    )
    .unwrap();

    assert!(query.select.where_clause.is_some());
}

#[test]
fn test_parse_vector_similarity_order() {
    let query = Parser::parse(
        "SELECT id, title FROM docs WHERE vector NEAR $v ORDER BY similarity(vector, $v) DESC LIMIT 5",
    )
    .unwrap();

    let order_by = query.select.order_by.expect("ORDER BY should be present");
    assert_eq!(order_by.len(), 1);
    assert!(
        matches!(&order_by[0].expr, OrderByExpr::Similarity(_)),
        "expected similarity(field, $param) to parse as OrderByExpr::Similarity, got {:?}",
        order_by[0].expr
    );
    assert!(
        order_by[0].descending,
        "ORDER BY ... DESC should set descending = true"
    );
}

// =============================================================================
// CATEGORY 5: Hybrid Vector + Aggregation Queries
// =============================================================================

#[test]
fn test_parse_vector_near_then_count() {
    // First filter by vector similarity, then count results per category
    let query = Parser::parse(
        "SELECT category, COUNT(*) FROM products WHERE vector NEAR $query GROUP BY category",
    )
    .unwrap();

    assert!(query.select.where_clause.is_some());
    assert!(query.select.group_by.is_some());
}

#[test]
fn test_parse_vector_search_with_aggregation_and_having() {
    let query = Parser::parse(
        "SELECT category, COUNT(*), AVG(price) FROM products \
         WHERE vector NEAR $embedding AND stock > 0 \
         GROUP BY category \
         HAVING COUNT(*) >= 3",
    )
    .unwrap();

    assert!(query.select.where_clause.is_some());
    assert!(query.select.group_by.is_some());
    assert!(query.select.having.is_some());
}

// =============================================================================
// CATEGORY 6: WITH Clause (Query Configuration)
// =============================================================================

#[test]
fn test_parse_with_ef_search() {
    let query =
        Parser::parse("SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 WITH (ef_search = 200)")
            .unwrap();

    assert!(query.select.with_clause.is_some());
    let with = query.select.with_clause.as_ref().unwrap();
    assert!(!with.options.is_empty());
}

#[test]
fn test_parse_with_multiple_options() {
    let query = Parser::parse(
        "SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 \
         WITH (ef_search = 200, rerank = true, threshold = 0.8)",
    )
    .unwrap();

    assert!(query.select.with_clause.is_some());
    let with = query.select.with_clause.as_ref().unwrap();
    assert!(with.options.len() >= 3);
}

// =============================================================================
// CATEGORY 7: JOIN Queries (Cross-Store)
// =============================================================================

#[test]
fn test_parse_simple_join() {
    let query =
        Parser::parse("SELECT * FROM products JOIN prices ON prices.product_id = products.id")
            .unwrap();

    assert!(!query.select.joins.is_empty());
    assert_eq!(query.select.joins[0].table, "prices");
}

#[test]
fn test_parse_join_with_alias() {
    let query =
        Parser::parse("SELECT * FROM products JOIN prices AS pr ON pr.product_id = products.id")
            .unwrap();

    assert!(!query.select.joins.is_empty());
    assert_eq!(query.select.joins[0].alias, Some("pr".to_string()));
}

#[test]
fn test_parse_multiple_joins() {
    let query = Parser::parse(
        "SELECT * FROM products \
         JOIN prices ON prices.product_id = products.id \
         JOIN inventory AS inv ON inv.product_id = products.id",
    )
    .unwrap();

    assert_eq!(query.select.joins.len(), 2);
}

// =============================================================================
// CATEGORY 8: Complex Combined Queries
// =============================================================================

#[test]
fn test_parse_full_featured_query() {
    // The "everything" query: vector + filter + group by + having + order + limit + with
    let query = Parser::parse(
        "SELECT category, COUNT(*), AVG(price) FROM products \
         WHERE vector NEAR $embedding AND stock > 0 \
         GROUP BY category \
         HAVING COUNT(*) > 5 \
         ORDER BY category \
         LIMIT 100 \
         OFFSET 10 \
         WITH (ef_search = 300)",
    )
    .unwrap();

    // Verify all clauses are parsed
    assert!(query.select.where_clause.is_some(), "Should have WHERE");
    assert!(query.select.group_by.is_some(), "Should have GROUP BY");
    assert!(query.select.having.is_some(), "Should have HAVING");
    assert!(query.select.order_by.is_some(), "Should have ORDER BY");
    assert_eq!(query.select.limit, Some(100), "Should have LIMIT 100");
    assert_eq!(query.select.offset, Some(10), "Should have OFFSET 10");
    assert!(query.select.with_clause.is_some(), "Should have WITH");
}

#[test]
fn test_parse_analytics_dashboard_query() {
    // Typical analytics query: aggregates with filters and grouping
    let query = Parser::parse(
        "SELECT region, product_type, SUM(revenue), COUNT(*), AVG(quantity) \
         FROM sales \
         WHERE date >= '2024-01-01' AND date <= '2024-12-31' \
         GROUP BY region, product_type \
         HAVING SUM(revenue) > 10000 \
         ORDER BY region \
         LIMIT 50",
    )
    .unwrap();

    assert!(query.select.group_by.is_some());
    let group_by = query.select.group_by.as_ref().unwrap();
    assert_eq!(group_by.columns.len(), 2);
}

#[test]
fn test_parse_semantic_search_with_metadata_filter() {
    // RAG-style query: vector similarity + metadata filters
    let query = Parser::parse(
        "SELECT id, title, content FROM documents \
         WHERE vector NEAR $query_embedding \
         AND category IN ('tech', 'science') \
         AND published = true \
         ORDER BY similarity(vector, $query_embedding) DESC \
         LIMIT 10 \
         WITH (ef_search = 400, threshold = 0.75)",
    )
    .unwrap();

    assert!(query.select.where_clause.is_some());
    assert!(query.select.order_by.is_some());
    assert!(query.select.with_clause.is_some());
}

// =============================================================================
// CATEGORY 9: EXPLAIN Query Plan
// =============================================================================

#[test]
fn test_explain_simple_scan() {
    let query = Parser::parse("SELECT * FROM products LIMIT 100").unwrap();
    let plan = QueryPlan::from_select(&query.select);

    // Should be a simple table scan
    assert!(plan.index_used.is_none());
    let tree = plan.to_tree();
    assert!(tree.contains("TableScan") || tree.contains("Scan"));
}

#[test]
fn test_explain_vector_search_uses_hnsw() {
    let query = Parser::parse("SELECT * FROM embeddings WHERE vector NEAR $v LIMIT 10").unwrap();
    let plan = QueryPlan::from_select(&query.select);

    // Should use HNSW index
    assert_eq!(
        plan.index_used,
        Some(crate::velesql::IndexType::Hnsw),
        "Vector search should use HNSW index"
    );
}

#[test]
fn test_explain_with_filter_shows_strategy() {
    let query =
        Parser::parse("SELECT * FROM products WHERE vector NEAR $v AND category = 'tech' LIMIT 10")
            .unwrap();
    let plan = QueryPlan::from_select(&query.select);

    // Should show filter strategy
    assert!(
        plan.filter_strategy != crate::velesql::FilterStrategy::None,
        "Should have a filter strategy"
    );
}

#[test]
fn test_explain_cost_estimation() {
    let query = Parser::parse("SELECT * FROM products LIMIT 1000").unwrap();
    let plan = QueryPlan::from_select(&query.select);

    // Cost should be positive
    assert!(plan.estimated_cost_ms > 0.0, "Cost should be positive");
}

#[test]
fn test_explain_to_json() {
    let query = Parser::parse("SELECT * FROM docs WHERE vector NEAR $v LIMIT 10").unwrap();
    let plan = QueryPlan::from_select(&query.select);
    let json = plan.to_json().expect("Should serialize to JSON");

    assert!(json.contains("\"estimated_cost_ms\""));
    assert!(json.contains("\"root\""));
}

// =============================================================================
// CATEGORY 10: Case Insensitivity (SQL Standard)
// =============================================================================

#[test]
fn test_case_insensitive_keywords() {
    // All these should parse identically
    let queries = [
        "SELECT * FROM docs WHERE vector NEAR $v LIMIT 10",
        "select * from docs where vector near $v limit 10",
        "Select * From docs Where vector Near $v Limit 10",
        "SELECT * FROM docs WHERE VECTOR NEAR $V LIMIT 10",
    ];

    let parsed: Vec<_> = queries
        .iter()
        .map(|sql| Parser::parse(sql).unwrap_or_else(|e| panic!("Failed to parse {sql}: {e:?}")))
        .collect();
    // Keywords (SELECT/FROM/WHERE/NEAR/LIMIT) are case-insensitive, so the
    // structural fields must be identical across variants. Identifiers and the
    // `$param` name preserve case, so we do NOT assert full-AST equality.
    for (sql, q) in queries.iter().zip(parsed.iter()) {
        assert_eq!(
            q.select.from, "docs",
            "FROM parsed case-insensitively: {sql}"
        );
        assert_eq!(
            q.select.limit,
            Some(10),
            "LIMIT parsed case-insensitively: {sql}"
        );
        assert!(
            q.select.where_clause.is_some(),
            "WHERE ... NEAR must parse into a condition: {sql}"
        );
    }
}

#[test]
fn test_case_insensitive_groupby_having() {
    let queries = [
        "SELECT category, COUNT(*) FROM items GROUP BY category HAVING COUNT(*) > 5",
        "select category, count(*) from items group by category having count(*) > 5",
        "Select category, Count(*) From items Group By category Having Count(*) > 5",
    ];

    for sql in &queries {
        let query = Parser::parse(sql).unwrap_or_else(|_| panic!("Failed: {}", sql));
        assert!(query.select.group_by.is_some());
        assert!(query.select.having.is_some());
    }
}

// =============================================================================
// CATEGORY 11: Edge Cases and Error Handling
// =============================================================================

#[test]
fn test_parse_empty_result_query() {
    // Simple query with filter that might return no results
    let query =
        Parser::parse("SELECT * FROM products WHERE stock = 0 LIMIT 10").expect("should parse");
    assert_eq!(query.select.from, "products");
    assert_eq!(query.select.limit, Some(10));
    assert!(
        query.select.where_clause.is_some(),
        "integer-equality WHERE should parse into a condition"
    );
}

#[test]
fn test_parse_very_long_column_list() {
    let columns = (1..=20)
        .map(|i| format!("col{}", i))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("SELECT {} FROM wide_table LIMIT 100", columns);

    let query = Parser::parse(&sql).expect("20-column SELECT should parse");
    match &query.select.columns {
        SelectColumns::Columns(cols) => {
            assert_eq!(cols.len(), 20, "all 20 columns must be preserved");
            assert_eq!(cols[0].name, "col1");
            assert_eq!(cols[19].name, "col20");
        }
        other => panic!("Expected a named-column list, got {other:?}"),
    }
}

#[test]
fn test_parse_nested_column_names() {
    // VelesQL supports single-level dot notation (table.column)
    let query = Parser::parse("SELECT payload.title, metadata.author FROM docs").unwrap();

    match &query.select.columns {
        SelectColumns::Columns(cols) => {
            assert!(cols.iter().any(|c| c.name.contains('.')));
        }
        _ => panic!("Expected columns"),
    }
}

#[test]
fn test_parse_special_characters_in_strings() {
    let query = Parser::parse("SELECT * FROM docs WHERE title = 'Hello, World!' LIMIT 10")
        .expect("query with comma+special chars in string literal should parse");
    assert_eq!(query.select.limit, Some(10));
    match query.select.where_clause {
        Some(Condition::Comparison(c)) => {
            assert_eq!(c.column, "title");
            // Comma inside the literal must NOT split the value; '!' must be preserved.
            assert_eq!(c.value, Value::String("Hello, World!".to_string()));
        }
        _ => panic!("Expected a Comparison WHERE condition"),
    }
}
