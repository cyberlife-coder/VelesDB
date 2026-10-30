use super::QueryResultKind;
use crate::velesql::Parser;

fn kind_of(sql: &str) -> QueryResultKind {
    Parser::parse(sql).expect("test: parse").result_kind()
}

#[test]
fn select_match_and_introspection_return_rows() {
    assert_eq!(kind_of("SELECT * FROM docs LIMIT 5"), QueryResultKind::Rows);
    assert_eq!(
        kind_of("MATCH (a:Person)-[:KNOWS]->(b) RETURN a, b LIMIT 5"),
        QueryResultKind::Rows
    );
    assert_eq!(kind_of("SHOW COLLECTIONS"), QueryResultKind::Rows);
}

#[test]
fn select_edges_returns_rows_not_a_mutation() {
    assert_eq!(
        kind_of("SELECT EDGES FROM kg WHERE source = 1"),
        QueryResultKind::Rows
    );
}

#[test]
fn row_affecting_dml_is_a_mutation() {
    assert_eq!(
        kind_of("INSERT INTO docs (id, title) VALUES (1, 'a')"),
        QueryResultKind::Mutation
    );
    assert_eq!(
        kind_of("UPDATE docs SET title = 'b' WHERE id = 1"),
        QueryResultKind::Mutation
    );
}

#[test]
fn delete_and_delete_edge_are_deletions() {
    assert_eq!(
        kind_of("DELETE FROM docs WHERE id = 1"),
        QueryResultKind::Deletion
    );
    assert_eq!(kind_of("DELETE EDGE 7 FROM kg"), QueryResultKind::Deletion);
}

#[test]
fn ddl_train_and_admin_have_their_own_kinds() {
    assert_eq!(
        kind_of("CREATE COLLECTION docs (dimension = 4, metric = 'cosine')"),
        QueryResultKind::Ddl
    );
    assert_eq!(
        kind_of("TRAIN QUANTIZER ON docs WITH (m = 8, k = 256)"),
        QueryResultKind::Train
    );
    assert_eq!(kind_of("FLUSH"), QueryResultKind::Admin);
}
