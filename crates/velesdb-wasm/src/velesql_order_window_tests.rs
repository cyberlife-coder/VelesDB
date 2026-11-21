//! `ORDER BY similarity(...)` ranks every row a browser statement admits
//! before `LIMIT` / `OFFSET` cut it, as core does on every route: a
//! metadata-filtered scan, a scan without WHERE, a named vector some rows do
//! not store (a null key: last under `ASC`, first under `DESC`), a `NEAR` or
//! a threshold read least similar first, the exact scan under `OR`, and a
//! MATCH whose nodes without an embedding have a null key.
//!
//! Each fixture holds 40 rows whose similarity to `$q` is a permutation of
//! their ids, so any prefix of the insertion order sorts to a wrong window.

use crate::database::DatabaseInner;
use crate::velesql_exec::execute;

const ROWS: u64 = 40;
const QUERY: &str = r#"{"q": [1.0, 0.0]}"#;

/// The rank of row `id` in similarity to `[1, 0]`: its vector is
/// `[1, rank / 10]`, so a smaller rank is more similar. `17 · id mod 41`
/// permutes `1..=40`, far from the id order.
fn rank(id: u64) -> u64 {
    (17 * id) % 41
}

fn vector(id: u64) -> String {
    #[allow(clippy::cast_precision_loss)]
    let y = rank(id) as f32 / 10.0;
    format!("[1.0, {y}]")
}

/// Whether row `id` stores no `emb` (`docs`) or no embedding (`kg`).
fn unscored(id: u64) -> bool {
    id.is_multiple_of(4)
}

fn ids(db: &mut DatabaseInner, sql: &str, params: &str) -> Vec<u64> {
    let result = execute(db, sql, Some(params)).unwrap_or_else(|e| panic!("test: {sql}: {e:?}"));
    (0..result.row_count() as usize)
        .map(|i| result.row(i).expect("test: row").id())
        .collect()
}

/// `docs`: 40 rows, `cat` alternating `'a'` (even ids) and `'b'`, each
/// row's named vector `emb` equal to its vector except on the rows
/// [`unscored`] names, which store none.
fn docs() -> DatabaseInner {
    let mut db = DatabaseInner::new();
    db.create_collection("docs", 2, "cosine")
        .expect("test: create docs");
    for id in 1..=ROWS {
        let cat = if id.is_multiple_of(2) { "a" } else { "b" };
        let sql = if unscored(id) {
            format!("INSERT INTO docs (id, vector, cat) VALUES ({id}, $v, '{cat}')")
        } else {
            format!("INSERT INTO docs (id, vector, cat, emb) VALUES ({id}, $v, '{cat}', $v)")
        };
        let params = format!(r#"{{"v": {}}}"#, vector(id));
        execute(&mut db, &sql, Some(&params)).expect("test: seed docs");
    }
    db
}

/// `kg`: 40 nodes, each storing its vector as its embedding except the
/// payload-only nodes [`unscored`] names.
fn kg() -> DatabaseInner {
    let mut db = DatabaseInner::new();
    db.create_collection("kg", 2, "cosine")
        .expect("test: create kg");
    for id in 1..=ROWS {
        if unscored(id) {
            let sql = format!("INSERT NODE INTO kg (id = {id}, payload = '{{\"name\":\"n\"}}')");
            execute(&mut db, &sql, None).expect("test: payload-only node");
        } else {
            let sql = format!("INSERT INTO kg (id, vector, name) VALUES ({id}, $v, 'n')");
            let params = format!(r#"{{"v": {}}}"#, vector(id));
            execute(&mut db, &sql, Some(&params)).expect("test: embedded node");
        }
    }
    db
}

/// The rows `admitted` ordered by similarity to `[1, 0]` — `scored` says
/// which rows have one — in `direction`, a null key last under `ASC` and
/// first under `DESC`, ties by ascending id; then `OFFSET`, then `LIMIT`.
fn expected(
    admitted: impl Iterator<Item = u64>,
    scored: impl Fn(u64) -> bool,
    descending: bool,
    offset: usize,
    limit: usize,
) -> Vec<u64> {
    let mut rows: Vec<u64> = admitted.collect();
    let key = |id: u64| std::cmp::Reverse(scored(id).then(|| rank(id)));
    rows.sort_by(|&a, &b| {
        let ascending = key(a).cmp(&key(b));
        let directed = if descending {
            ascending.reverse()
        } else {
            ascending
        };
        directed.then(a.cmp(&b))
    });
    rows.into_iter().skip(offset).take(limit).collect()
}

const WINDOWS: [(&str, usize); 4] = [("ASC", 0), ("ASC", 2), ("DESC", 0), ("DESC", 2)];

fn assert_windows(
    tail: &str,
    key: &str,
    admitted: &dyn Fn(u64) -> bool,
    scored: &dyn Fn(u64) -> bool,
) {
    let mut db = docs();
    for (direction, offset) in WINDOWS {
        let sql = format!("SELECT * {tail} ORDER BY {key} {direction} LIMIT 3 OFFSET {offset}");
        let want = expected(
            (1..=ROWS).filter(|&id| admitted(id)),
            scored,
            direction == "DESC",
            offset,
            3,
        );
        assert_eq!(ids(&mut db, &sql, QUERY), want, "{sql}");
    }
}

#[test]
fn a_metadata_filtered_scan_ranks_every_admitted_row() {
    assert_windows(
        "FROM docs WHERE cat = 'a'",
        "similarity(vector, $q)",
        &|id| id.is_multiple_of(2),
        &|_| true,
    );
}

#[test]
fn a_scan_without_where_ranks_every_row() {
    assert_windows("FROM docs", "similarity(vector, $q)", &|_| true, &|_| true);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn a_named_vector_ranks_rows_without_one_as_null_keys() {
    assert_windows(
        "FROM docs WHERE id > 0",
        "similarity(emb, $q)",
        &|_| true,
        &|id| !unscored(id),
    );
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn equal_null_keys_keep_ascending_ids_in_both_directions() {
    let mut db = docs();
    for direction in ["ASC", "DESC"] {
        let sql = format!(
            "SELECT * FROM docs WHERE id IN (16, 4, 12, 8) \
             ORDER BY similarity(emb, $q) {direction} LIMIT 2 OFFSET 1"
        );
        assert_eq!(ids(&mut db, &sql, QUERY), [8, 12], "{sql}");
        assert_eq!(
            expected(
                [16, 4, 12, 8].into_iter(),
                |id| !unscored(id),
                direction == "DESC",
                1,
                2
            ),
            [8, 12],
            "oracle for {direction}"
        );
    }
}

#[test]
fn a_near_read_in_another_order_ranks_every_admitted_row() {
    let mut db = docs();
    let sql = "SELECT * FROM docs WHERE vector NEAR $q ORDER BY similarity() ASC LIMIT 3";
    assert_eq!(
        ids(&mut db, sql, QUERY),
        expected(1..=ROWS, |_| true, false, 0, 3),
        "{sql}"
    );
    let sql =
        "SELECT * FROM docs WHERE vector NEAR $q ORDER BY similarity(vector, $far) DESC LIMIT 3";
    let params = r#"{"q": [1.0, 0.0], "far": [0.0, 1.0]}"#;
    assert_eq!(
        ids(&mut db, sql, params),
        expected(1..=ROWS, |_| true, false, 0, 3),
        "{sql}"
    );
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn a_fused_order_reads_beyond_the_small_windows_branch_depth() {
    let mut db = docs();
    let shallow = velesdb_core::fusion::branch_depth(1);
    assert!(usize::try_from(ROWS).expect("test: row count") > shallow);
    let sql = "SELECT * FROM docs WHERE vector NEAR_FUSED [$q, $q] \
               ORDER BY similarity() ASC LIMIT 1";
    assert_eq!(
        ids(&mut db, sql, QUERY),
        expected(1..=ROWS, |_| true, false, 0, 1)
    );
}

#[test]
fn a_threshold_read_least_similar_first_ranks_every_admitted_row() {
    assert_windows(
        "FROM docs WHERE similarity(vector, $q) > 0.0",
        "similarity(vector, $q)",
        &|_| true,
        &|_| true,
    );
}

#[test]
fn the_exact_similarity_scan_ranks_every_admitted_row() {
    assert_windows(
        "FROM docs WHERE cat = 'a' OR similarity(emb, $q) > 0.0",
        "similarity(emb, $q)",
        &|id| id.is_multiple_of(2) || !unscored(id),
        &|id| !unscored(id),
    );
}

#[test]
fn a_match_ranks_every_row_before_its_limit() {
    let mut db = kg();
    let params = r#"{"_collection": "kg", "q": [1.0, 0.0]}"#;
    for direction in ["ASC", "DESC"] {
        let descending = direction == "DESC";
        let sql =
            format!("MATCH (a) RETURN a ORDER BY similarity(a.vector, $q) {direction} LIMIT 3");
        let want = expected(1..=ROWS, |id| !unscored(id), descending, 0, 3);
        assert_eq!(ids(&mut db, &sql, params), want, "{sql}");

        let sql = format!(
            "MATCH (a) WHERE similarity(a.vector, $q) > 0.0 RETURN a \
             ORDER BY similarity() {direction} LIMIT 3"
        );
        let scored = (1..=ROWS).filter(|&id| !unscored(id));
        let want = expected(scored, |_| true, descending, 0, 3);
        assert_eq!(ids(&mut db, &sql, params), want, "{sql}");
    }
}
