//! Native tests of the key scheme. The IndexedDB paths themselves run in a
//! browser: see `tests/idb_persistence_web.rs`.

use super::{is_row_of, row_key};

#[test]
fn a_row_key_belongs_to_its_graph_only() {
    assert!(is_row_of(&row_key("a", 7), "a"));
    assert!(is_row_of(&row_key("a:b", 2), "a:b"));
    assert!(
        !is_row_of(&row_key("a:b", 2), "a"),
        "graph a:b's row is not graph a's"
    );
    assert!(!is_row_of("a:", "a"), "no id");
    assert!(!is_row_of("ab:1", "a"), "another name");
}
