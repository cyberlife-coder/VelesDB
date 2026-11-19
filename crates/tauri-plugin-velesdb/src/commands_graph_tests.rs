use super::build_edge;

#[test]
fn test_build_edge_with_object_properties() {
    let edge = build_edge(1, 10, 20, "KNOWS", Some(serde_json::json!({"weight": 0.5})))
        .expect("valid edge");
    assert_eq!(edge.id(), 1);
    assert_eq!(edge.source(), 10);
    assert_eq!(edge.target(), 20);
    assert_eq!(edge.label(), "KNOWS");
}

#[test]
fn test_build_edge_null_or_absent_properties_mean_none() {
    assert!(build_edge(2, 1, 2, "L", None).is_ok());
    assert!(build_edge(3, 1, 2, "L", Some(serde_json::Value::Null)).is_ok());
}

/// Properties that are neither an object nor null are refused, as REST
/// refuses them — they used to be dropped silently.
#[test]
fn test_build_edge_refuses_non_object_properties() {
    for bad in [
        serde_json::json!("scalar"),
        serde_json::json!([1, 2]),
        serde_json::json!(7),
        serde_json::json!(true),
    ] {
        let err = build_edge(4, 1, 2, "L", Some(bad.clone()))
            .expect_err("non-object properties are refused");
        assert!(
            matches!(err, crate::error::Error::InvalidConfig(_)),
            "{bad}: {err}"
        );
        assert!(
            err.to_string()
                .contains("Properties must be an object or null"),
            "{err}"
        );
    }
}

/// `get_edges` keeps only the edges matching every filter given — it used
/// to apply the first of label / source / target and drop the others.
#[test]
fn test_edges_matching_every_filter_given() {
    let edge = |id, s, t, l| velesdb_core::GraphEdge::new(id, s, t, l).expect("test: edge");
    let edges = vec![
        edge(1, 42, 7, "FOLLOWS"),
        edge(2, 42, 8, "LIKES"),
        edge(3, 9, 7, "FOLLOWS"),
    ];
    let ids = |label, source, target| {
        super::keep_matching(edges.clone(), label, source, target)
            .iter()
            .map(velesdb_core::GraphEdge::id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(Some("FOLLOWS"), Some(42), None), vec![1]);
    assert_eq!(ids(Some("FOLLOWS"), None, Some(7)), vec![1, 3]);
    assert_eq!(ids(None, Some(42), Some(8)), vec![2]);
    assert_eq!(ids(None, None, None), vec![1, 2, 3]);
}

/// An invalid edge label keeps core's own code (VELES-021), not the
/// caller-configuration code it used to be flattened to (VELES-009).
#[test]
fn test_an_invalid_edge_label_keeps_cores_code() {
    let err = super::build_edge(1, 1, 2, "", None).expect_err("an empty label is refused");
    assert_eq!(crate::error::CommandError::from(err).code, "VELES-021");
}

/// The algorithm is matched case-insensitively against `bfs` / `dfs`, as the
/// REST traverse endpoint does — anything else is refused instead of
/// silently running BFS.
#[test]
fn test_traversal_algorithm_is_parsed_like_rest() {
    use super::{parse_algorithm, TraversalAlgorithm};
    assert_eq!(parse_algorithm("bfs").ok(), Some(TraversalAlgorithm::Bfs));
    assert_eq!(parse_algorithm("BFS").ok(), Some(TraversalAlgorithm::Bfs));
    assert_eq!(parse_algorithm("dfs").ok(), Some(TraversalAlgorithm::Dfs));
    assert_eq!(parse_algorithm("DFS").ok(), Some(TraversalAlgorithm::Dfs));
    for unknown in ["dijkstra", "", "b f s"] {
        let err = parse_algorithm(unknown).expect_err("an unknown algorithm is refused");
        assert!(
            matches!(err, crate::error::Error::InvalidConfig(_)),
            "{err}"
        );
        assert!(err.to_string().contains("Use 'bfs' or 'dfs'"), "{err}");
    }
}
