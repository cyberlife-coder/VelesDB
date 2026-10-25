use super::*;
use std::collections::HashMap;

fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let table: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
    move |key| table.get(key).cloned()
}

#[test]
fn test_ci_detection_table() {
    let cases: &[(&[(&str, &str)], bool)] = &[
        (&[], false),
        (&[("CI", "true")], true),
        (&[("CI", "1")], true),
        (&[("CI", "TRUE")], true),
        (&[("CI", "false")], false),
        (&[("CI", "0")], false),
        (&[("CI", "")], false),
        (&[("GITHUB_ACTIONS", "true")], true),
        (&[("GITLAB_CI", "true")], true),
        (&[("BUILDKITE", "true")], true),
        (&[("CIRCLECI", "true")], true),
        (&[("TF_BUILD", "True")], true),
        (&[("JENKINS_URL", "https://ci.example.com/")], true),
        (&[("TEAMCITY_VERSION", "2024.12")], true),
        (&[("TRAVIS", "true")], true),
        (&[("APPVEYOR", "True")], true),
        (&[("GITHUB_ACTIONS", "")], false),
        (&[("HOME", "/home/dev"), ("TERM", "xterm")], false),
        (&[("CI", "false"), ("GITHUB_ACTIONS", "true")], true),
        (&[("CI", "no")], false),
        (&[("CI", "off")], false),
        (&[("CI", "OFF")], false),
        (&[("TRAVIS", "false")], false),
        (&[("APPVEYOR", "False")], false),
        (&[("GITLAB_CI", "0")], false),
        (&[("CI", "no"), ("TRAVIS", "false")], false),
    ];
    for (pairs, expected) in cases {
        assert_eq!(is_ci(env(pairs)), *expected, "environment {pairs:?}");
    }
}
