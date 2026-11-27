use super::parse_rel_types;

#[test]
fn parse_rel_types_drops_blank_entries() {
    assert!(parse_rel_types("").is_empty());
    assert!(parse_rel_types(" , ,").is_empty());
    assert_eq!(parse_rel_types(" KNOWS, ,FOLLOWS,"), ["KNOWS", "FOLLOWS"]);
}
