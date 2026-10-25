//! Continuous-integration detection.
//!
//! A CI runner starts a fresh process on a fresh machine for every job, so
//! each run looks like a new install. The update check reports whether it
//! runs under CI so the server can keep those out of its install counts.

use super::config::is_falsy;

/// Variables whose presence, with a value that is not a falsy word, identifies
/// a CI system.
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
    std::iter::once("CI")
        .chain(CI_MARKERS.iter().copied())
        .any(|name| lookup(name).is_some_and(|value| is_switched_on(&value)))
}

/// `CI=true`, `CI=1` and a non-empty `JENKINS_URL` count; an empty value and
/// the usual falsy words (`0`, `false`, `no`, `off`) are how a shell turns a
/// flag off, and `TRAVIS=false` is not a Travis run.
fn is_switched_on(value: &str) -> bool {
    !value.is_empty() && !is_falsy(value)
}

#[cfg(test)]
#[path = "ci_tests.rs"]
mod tests;
