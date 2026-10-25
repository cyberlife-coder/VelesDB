//! Continuous-integration detection.
//!
//! A CI runner starts a fresh process on a fresh machine for every job, so
//! each run looks like a new install. The update check reports whether it
//! runs under CI so the server can keep those out of its install counts.

/// Variables whose mere presence (non-empty) identifies a CI system.
const CI_MARKERS: &[&str] = &[
    "GITHUB_ACTIONS",
    "GITLAB_CI",
    "BUILDKITE",
    "CIRCLECI",
    "TF_BUILD",
    "JENKINS_URL",
    "TEAMCITY_VERSION",
    "TRAVIS",
    "APPVEYOR",
    "DRONE",
    "BITBUCKET_BUILD_NUMBER",
    "CODEBUILD_BUILD_ID",
];

/// Returns `true` when the environment looks like a CI runner.
///
/// `lookup` reads one environment variable, so tests pass a table instead of
/// mutating the process environment.
pub(super) fn is_ci(lookup: impl Fn(&str) -> Option<String>) -> bool {
    let ci_flag = lookup("CI").is_some_and(|v| is_set_and_true(&v));
    ci_flag
        || CI_MARKERS
            .iter()
            .any(|name| lookup(name).is_some_and(|v| !v.is_empty()))
}

/// `CI=true`, `CI=1` and the like; `CI=false`, `CI=0` and an empty value are
/// the way a shell turns the flag off.
fn is_set_and_true(value: &str) -> bool {
    !matches!(value.to_ascii_lowercase().as_str(), "" | "0" | "false")
}

#[cfg(test)]
#[path = "ci_tests.rs"]
mod tests;
