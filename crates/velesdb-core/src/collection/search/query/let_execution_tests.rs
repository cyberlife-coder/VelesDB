/// Issue #473: `LET hybrid = 0.5 SELECT docs.*, hybrid FROM docs`
/// Tests the qualified-wildcard mixed SELECT case — the LET binding and
/// wildcard-expanded fields must both appear in the payload after projection.
#[test]
fn test_let_binding_in_qualified_wildcard_mixed_select() {