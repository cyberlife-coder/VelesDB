//! `\show` prints the value in force, and an unset mode shows the default the
//! database was opened with (#2303).

use tempfile::TempDir;

use super::shown_settings;
use crate::session::SessionSettings;
use crate::test_fixtures::seed_docs_configured;

#[test]
fn show_prints_the_configured_default_for_an_unset_mode() {
    let dir = TempDir::new().expect("test: temp dir");
    let db = seed_docs_configured(&dir, "[search]\ndefault_mode = \"fast\"\n", 1);
    let untouched = SessionSettings::new();
    assert_eq!(
        shown_settings(&db, &untouched, Some("mode")),
        Ok(vec![(
            "mode".to_string(),
            "fast (configured default)".to_string()
        )])
    );
    let all = shown_settings(&db, &untouched, None).expect("test: every setting");
    assert!(
        all.contains(&("mode".to_string(), "fast (configured default)".to_string())),
        "{all:?}"
    );

    let mut set = SessionSettings::new();
    set.set("mode", "accurate").expect("test: \\set mode");
    assert_eq!(
        shown_settings(&db, &set, Some("mode")),
        Ok(vec![("mode".to_string(), "accurate".to_string())])
    );
    assert_eq!(
        shown_settings(&db, &set, Some("nope")),
        Err("Unknown setting: nope".to_string())
    );
}
