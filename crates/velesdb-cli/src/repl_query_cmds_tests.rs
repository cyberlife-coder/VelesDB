use super::*;

#[test]
fn bare_dollar_is_detected() {
    assert!(contains_unquoted_dollar("SELECT * FROM t WHERE id = $id"));
}

#[test]
fn dollar_inside_single_quoted_string_is_ignored() {
    assert!(!contains_unquoted_dollar(
        "SELECT * FROM t WHERE name = '$foo'"
    ));
}

#[test]
fn dollar_outside_quotes_with_quoted_string_present() {
    assert!(contains_unquoted_dollar(
        "SELECT * FROM t WHERE name = '$foo' AND id = $id"
    ));
}

#[test]
fn no_dollar_at_all() {
    assert!(!contains_unquoted_dollar(
        "SELECT * FROM t WHERE name = 'hello'"
    ));
}

#[test]
fn multiple_quoted_strings_no_bare_dollar() {
    assert!(!contains_unquoted_dollar(
        "SELECT * FROM t WHERE a = '$x' AND b = '$y'"
    ));
}

#[test]
fn empty_string() {
    assert!(!contains_unquoted_dollar(""));
}
