//! Which recall quality an operator asked for, and what is refused.
//!
//! The grammar is the one every other surface speaks
//! (`velesdb_core::api_types::parse_search_mode`), so these tests pin only what
//! this crate adds: that "not configured" is distinct from every quality, and
//! that a bad value is the operator's to correct, never silently the default.

use super::*;
use crate::SearchQuality;

#[test]
fn nothing_configured_leaves_the_collections_own_quality() {
    assert_eq!(select_search_quality(None), Ok(None));
}

#[test]
fn a_blank_value_is_the_same_as_nothing() {
    assert_eq!(select_search_quality(Some("")), Ok(None));
    assert_eq!(select_search_quality(Some("   ")), Ok(None));
}

#[test]
fn every_named_quality_is_selectable() {
    for (name, expected) in [
        ("fast", SearchQuality::Fast),
        ("balanced", SearchQuality::Balanced),
        ("accurate", SearchQuality::Accurate),
        ("perfect", SearchQuality::Perfect),
        ("autotune", SearchQuality::AutoTune),
    ] {
        assert_eq!(
            select_search_quality(Some(name)),
            Ok(Some(expected)),
            "{name}"
        );
    }
}

#[test]
fn the_spelling_is_forgiven_like_everywhere_else() {
    assert_eq!(
        select_search_quality(Some(" Accurate ")),
        Ok(Some(SearchQuality::Accurate))
    );
}

#[test]
fn an_explicit_ef_is_selectable() {
    assert_eq!(
        select_search_quality(Some("custom:256")),
        Ok(Some(SearchQuality::Custom(256)))
    );
}

#[test]
fn an_unknown_quality_is_refused_with_the_accepted_forms() {
    let error = select_search_quality(Some("turbo")).expect_err("not a quality");

    assert!(
        matches!(&error, SettingError::InvalidSearchQuality { detail } if detail.contains("turbo")),
        "{error:?}"
    );
    assert!(error.to_string().contains("fast"), "{error}");
}
