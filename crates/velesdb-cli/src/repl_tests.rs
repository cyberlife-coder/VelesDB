use super::*;
use crate::repl_execute::contains_param_vector;
use serde_json::json;
use velesdb_core::velesql::{
    CompareOp, Comparison, Condition, FusionConfig, SimilarityCondition, SparseVectorExpr,
    SparseVectorSearch, Value, VectorExpr, VectorFusedSearch, VectorSearch,
};

fn contains_vector_search(condition: &velesdb_core::velesql::Condition) -> bool {
    use velesdb_core::velesql::Condition;
    match condition {
        Condition::VectorSearch(_) => true,
        Condition::And(left, right) | Condition::Or(left, right) => {
            contains_vector_search(left) || contains_vector_search(right)
        }
        Condition::Group(inner) => contains_vector_search(inner),
        _ => false,
    }
}

// =========================================================================
// Tests for ReplConfig
// =========================================================================

#[test]
fn test_repl_config_default() {
    let config = ReplConfig::default();
    assert!(config.timing);
    assert_eq!(config.format, OutputFormat::Table);
}

#[test]
fn test_output_format_eq() {
    assert_eq!(OutputFormat::Table, OutputFormat::Table);
    assert_eq!(OutputFormat::Json, OutputFormat::Json);
    assert_ne!(OutputFormat::Table, OutputFormat::Json);
}

// =========================================================================
// Tests for QueryResult
// =========================================================================

#[test]
fn test_query_result_empty() {
    let result = QueryResult {
        rows: vec![],
        duration_ms: 0.0,
        kind: QueryKind::Select,
    };
    assert!(result.rows.is_empty());
    assert!((result.duration_ms - 0.0).abs() < f64::EPSILON);
}

#[test]
fn test_query_result_with_data() {
    let mut row = HashMap::new();
    row.insert("id".to_string(), json!(1));
    row.insert("name".to_string(), json!("test"));

    let result = QueryResult {
        rows: vec![row],
        duration_ms: 1.5,
        kind: QueryKind::Select,
    };

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].get("id"), Some(&json!(1)));
    assert!((result.duration_ms - 1.5).abs() < f64::EPSILON);
}

#[test]
fn test_query_kind_ddl() {
    let result = QueryResult {
        rows: vec![],
        duration_ms: 0.5,
        kind: QueryKind::Ddl,
    };
    assert_eq!(result.kind, QueryKind::Ddl);
}

#[test]
fn test_query_kind_dml() {
    let result = QueryResult {
        rows: vec![],
        duration_ms: 0.3,
        kind: QueryKind::Dml,
    };
    assert_eq!(result.kind, QueryKind::Dml);
}

// =========================================================================
// Tests for contains_vector_search
// =========================================================================

#[test]
fn test_contains_vector_search_with_vector() {
    let condition = Condition::VectorSearch(VectorSearch {
        vector: VectorExpr::Literal(vec![0.1, 0.2]),
    });
    assert!(contains_vector_search(&condition));
}

#[test]
fn test_contains_vector_search_without_vector() {
    let condition = Condition::Comparison(Comparison {
        column: "category".to_string(),
        operator: CompareOp::Eq,
        value: Value::String("tech".to_string()),
    });
    assert!(!contains_vector_search(&condition));
}

#[test]
fn test_contains_vector_search_nested_and() {
    let vector_cond = Condition::VectorSearch(VectorSearch {
        vector: VectorExpr::Literal(vec![0.1]),
    });
    let other_cond = Condition::Comparison(Comparison {
        column: "x".to_string(),
        operator: CompareOp::Eq,
        value: Value::Integer(1),
    });
    let combined = Condition::And(Box::new(other_cond), Box::new(vector_cond));
    assert!(contains_vector_search(&combined));
}

#[test]
fn test_contains_vector_search_nested_or() {
    let vector_cond = Condition::VectorSearch(VectorSearch {
        vector: VectorExpr::Literal(vec![0.1]),
    });
    let other_cond = Condition::Comparison(Comparison {
        column: "x".to_string(),
        operator: CompareOp::Eq,
        value: Value::Integer(1),
    });
    let combined = Condition::Or(Box::new(other_cond), Box::new(vector_cond));
    assert!(contains_vector_search(&combined));
}

#[test]
fn test_contains_vector_search_group() {
    let vector_cond = Condition::VectorSearch(VectorSearch {
        vector: VectorExpr::Literal(vec![0.1]),
    });
    let grouped = Condition::Group(Box::new(vector_cond));
    assert!(contains_vector_search(&grouped));
}

#[test]
fn test_contains_vector_search_no_match() {
    let cond_a = Condition::Comparison(Comparison {
        column: "a".to_string(),
        operator: CompareOp::Eq,
        value: Value::Integer(1),
    });
    let cond_b = Condition::Comparison(Comparison {
        column: "b".to_string(),
        operator: CompareOp::Gt,
        value: Value::Integer(2),
    });
    let condition = Condition::And(Box::new(cond_a), Box::new(cond_b));
    assert!(!contains_vector_search(&condition));
}

// =========================================================================
// Tests for contains_param_vector (Phase 1.1 -- exhaustive variants)
// =========================================================================

#[test]
fn test_contains_param_vector_vector_search_param() {
    let cond = Condition::VectorSearch(VectorSearch {
        vector: VectorExpr::Parameter("v".to_string()),
    });
    assert!(contains_param_vector(&cond));
}

#[test]
fn test_contains_param_vector_vector_search_literal() {
    let cond = Condition::VectorSearch(VectorSearch {
        vector: VectorExpr::Literal(vec![0.1, 0.2]),
    });
    assert!(!contains_param_vector(&cond));
}

#[test]
fn test_contains_param_vector_fused_search_param() {
    let cond = Condition::VectorFusedSearch(VectorFusedSearch {
        vectors: vec![
            VectorExpr::Literal(vec![0.1]),
            VectorExpr::Parameter("q".to_string()),
        ],
        fusion: FusionConfig::default(),
    });
    assert!(contains_param_vector(&cond));
}

#[test]
fn test_contains_param_vector_fused_search_all_literal() {
    let cond = Condition::VectorFusedSearch(VectorFusedSearch {
        vectors: vec![
            VectorExpr::Literal(vec![0.1]),
            VectorExpr::Literal(vec![0.2]),
        ],
        fusion: FusionConfig::default(),
    });
    assert!(!contains_param_vector(&cond));
}

#[test]
fn test_contains_param_vector_sparse_search_param() {
    let cond = Condition::SparseVectorSearch(SparseVectorSearch {
        vector: SparseVectorExpr::Parameter("sv".to_string()),
        index_name: None,
    });
    assert!(contains_param_vector(&cond));
}

#[test]
fn test_contains_param_vector_sparse_search_literal() {
    use velesdb_core::sparse_index::SparseVector;
    let cond = Condition::SparseVectorSearch(SparseVectorSearch {
        vector: SparseVectorExpr::Literal(SparseVector::new(vec![(0, 1.0)])),
        index_name: None,
    });
    assert!(!contains_param_vector(&cond));
}

#[test]
fn test_contains_param_vector_similarity_param() {
    let cond = Condition::Similarity(SimilarityCondition {
        field: "embedding".to_string(),
        vector: VectorExpr::Parameter("q".to_string()),
        operator: CompareOp::Gt,
        threshold: 0.8,
    });
    assert!(contains_param_vector(&cond));
}

#[test]
fn test_contains_param_vector_similarity_literal() {
    let cond = Condition::Similarity(SimilarityCondition {
        field: "embedding".to_string(),
        vector: VectorExpr::Literal(vec![0.1, 0.2]),
        operator: CompareOp::Gt,
        threshold: 0.8,
    });
    assert!(!contains_param_vector(&cond));
}

#[test]
fn test_contains_param_vector_not_recurses() {
    let inner = Condition::VectorSearch(VectorSearch {
        vector: VectorExpr::Parameter("v".to_string()),
    });
    let cond = Condition::Not(Box::new(inner));
    assert!(contains_param_vector(&cond));
}

#[test]
fn test_contains_param_vector_not_no_param() {
    let inner = Condition::Comparison(Comparison {
        column: "x".to_string(),
        operator: CompareOp::Eq,
        value: Value::Integer(1),
    });
    let cond = Condition::Not(Box::new(inner));
    assert!(!contains_param_vector(&cond));
}

#[test]
fn test_contains_param_vector_comparison_false() {
    let cond = Condition::Comparison(Comparison {
        column: "cat".to_string(),
        operator: CompareOp::Eq,
        value: Value::String("tech".to_string()),
    });
    assert!(!contains_param_vector(&cond));
}

#[test]
fn test_contains_param_vector_nested_and_with_similarity() {
    let sim = Condition::Similarity(SimilarityCondition {
        field: "vec".to_string(),
        vector: VectorExpr::Parameter("q".to_string()),
        operator: CompareOp::Gt,
        threshold: 0.5,
    });
    let comp = Condition::Comparison(Comparison {
        column: "status".to_string(),
        operator: CompareOp::Eq,
        value: Value::String("active".to_string()),
    });
    let combined = Condition::And(Box::new(sim), Box::new(comp));
    assert!(contains_param_vector(&combined));
}

// =========================================================================
// Tests for print_result (output format logic)
// =========================================================================

#[test]
fn test_print_result_empty() {
    let result = QueryResult {
        rows: vec![],
        duration_ms: 0.0,
        kind: QueryKind::Select,
    };
    // Should not panic on empty results
    print_result(&result, "table");
    print_result(&result, "json");
}

#[test]
fn test_print_result_json_format() {
    let mut row = HashMap::new();
    row.insert("id".to_string(), json!(1));

    let result = QueryResult {
        rows: vec![row],
        duration_ms: 1.0,
        kind: QueryKind::Select,
    };
    // Should not panic
    print_result(&result, "json");
}

#[test]
fn test_print_result_table_format() {
    let mut row = HashMap::new();
    row.insert("id".to_string(), json!(42));
    row.insert("name".to_string(), json!("test"));

    let result = QueryResult {
        rows: vec![row],
        duration_ms: 2.0,
        kind: QueryKind::Select,
    };
    // Should not panic
    print_result(&result, "table");
}
