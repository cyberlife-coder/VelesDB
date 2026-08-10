//! Behaviour of the TOML config file and its precedence rule.

use super::*;

/// Write `text` to a temp file and return the dir (which must outlive it).
fn config_file(text: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(CONFIG_FILE_NAME);
    std::fs::write(&path, text).expect("write config");
    (dir, path)
}

#[test]
fn every_documented_knob_maps_to_its_variable() {
    let (_dir, path) = config_file(
        r#"
path = "/tmp/store"
quiet = true
default_ttl = 3600

[http]
enabled = true
bind = "127.0.0.1:18090"
insecure = false
allow_remote = false
max_body_bytes = 1048576
max_sessions = 64
tls_dir = "/tmp/tls"

[embedder]
backend = "ollama"
model = "bge-m3"
url = "http://localhost:11434"
keep_alive = "30m"

[extractor]
backend = "ollama"
model = "qwen3.6:35b-mlx"
url = "http://localhost:11434"
"#,
    );
    let loaded = load(&path).expect("valid config");
    let v = &loaded.values;
    assert_eq!(v["VELESDB_MEMORY_PATH"], "/tmp/store");
    assert_eq!(v["VELESDB_MEMORY_QUIET"], "1");
    assert_eq!(v["VELESDB_MEMORY_DEFAULT_TTL"], "3600");
    assert_eq!(v["VELESDB_MEMORY_HTTP"], "1");
    assert_eq!(v["VELESDB_MEMORY_HTTP_BIND"], "127.0.0.1:18090");
    assert_eq!(v["VELESDB_MEMORY_HTTP_INSECURE"], "0");
    assert_eq!(v["VELESDB_MEMORY_HTTP_ALLOW_REMOTE"], "0");
    assert_eq!(v["VELESDB_MEMORY_HTTP_MAX_BODY_BYTES"], "1048576");
    assert_eq!(v["VELESDB_MEMORY_HTTP_MAX_SESSIONS"], "64");
    assert_eq!(v["VELESDB_MEMORY_TLS_DIR"], "/tmp/tls");
    assert_eq!(v["VELESDB_MEMORY_EMBEDDER"], "ollama");
    // The file is role-named (`[embedder]`), so it writes the role-named
    // variables. The `VELESDB_MEMORY_OLLAMA_*` pair survives as an environment
    // alias for anyone who set it in a shell or a launch plist, but nothing
    // written here produces it — a file that emitted a product name for a
    // section named after a role is the asymmetry #1751 exists to close.
    assert_eq!(v["VELESDB_MEMORY_EMBEDDER_MODEL"], "bge-m3");
    assert_eq!(v["VELESDB_MEMORY_EMBEDDER_URL"], "http://localhost:11434");
    // `keep_alive` keeps the product name, and legitimately: it is a field of
    // Ollama's own wire protocol, not a role-level setting an OpenAI-compatible
    // server would know what to do with.
    assert_eq!(v["VELESDB_MEMORY_OLLAMA_KEEP_ALIVE"], "30m");
    assert_eq!(v["VELESDB_MEMORY_EXTRACTOR"], "ollama");
    assert_eq!(v["VELESDB_MEMORY_EXTRACTOR_MODEL"], "qwen3.6:35b-mlx");
    assert_eq!(v["VELESDB_MEMORY_EXTRACTOR_URL"], "http://localhost:11434");
}

/// The eighteen variables the binary reads, minus the two that are not file
/// settings (`VELESDB_MEMORY_CONFIG` selects the file itself; the store path
/// is covered above). Guards against a knob being added to the binary and
/// forgotten here — the whole promise is that the file covers everything.
#[test]
fn the_file_covers_every_configurable_variable() {
    let (_dir, path) = config_file(
        r#"
path = "/tmp/s"
quiet = false
default_ttl = 1
[http]
enabled = false
bind = "x"
insecure = true
allow_remote = true
max_body_bytes = 1
max_sessions = 1
tls_dir = "t"
[embedder]
backend = "hash"
model = "m"
url = "u"
keep_alive = "k"
[extractor]
backend = "ollama"
model = "m"
url = "u"
[context]
ingest_roots = ["/tmp"]
[graph]
autograph = true
"#,
    );
    let loaded = load(&path).expect("valid config");
    let expected = [
        "VELESDB_MEMORY_PATH",
        "VELESDB_MEMORY_QUIET",
        "VELESDB_MEMORY_DEFAULT_TTL",
        "VELESDB_MEMORY_HTTP",
        "VELESDB_MEMORY_HTTP_BIND",
        "VELESDB_MEMORY_HTTP_INSECURE",
        "VELESDB_MEMORY_HTTP_ALLOW_REMOTE",
        "VELESDB_MEMORY_HTTP_MAX_BODY_BYTES",
        "VELESDB_MEMORY_HTTP_MAX_SESSIONS",
        "VELESDB_MEMORY_TLS_DIR",
        "VELESDB_MEMORY_EMBEDDER",
        "VELESDB_MEMORY_EMBEDDER_MODEL",
        "VELESDB_MEMORY_EMBEDDER_URL",
        "VELESDB_MEMORY_OLLAMA_KEEP_ALIVE",
        "VELESDB_MEMORY_EXTRACTOR",
        "VELESDB_MEMORY_EXTRACTOR_MODEL",
        "VELESDB_MEMORY_EXTRACTOR_URL",
        "VELESDB_MEMORY_INGEST_ROOTS",
        "VELESDB_MEMORY_AUTOGRAPH",
    ];
    for key in expected {
        assert!(
            loaded.values.contains_key(key),
            "{key} is not settable from the config file"
        );
    }
}

#[test]
fn a_false_flag_is_written_not_omitted() {
    let (_dir, path) = config_file("[http]\ninsecure = false\n");
    let loaded = load(&path).expect("valid config");
    assert_eq!(
        loaded.values["VELESDB_MEMORY_HTTP_INSECURE"], "0",
        "writing `false` must hold the setting off, not fall through to a default"
    );
}

#[test]
fn an_empty_file_sets_nothing_and_is_not_an_error() {
    let (_dir, path) = config_file("");
    assert!(load(&path).expect("empty is valid").values.is_empty());
}

// --- C1: the role-named variable and its legacy alias ----------------------

#[test]
fn the_role_named_variable_wins_over_its_legacy_alias() {
    let resolved = resolve_alias(Some("http://role"), Some("http://legacy"));
    assert_eq!(resolved.value.as_deref(), Some("http://role"));
    assert!(
        resolved.conflicting,
        "two different values is exactly the case the operator must be told about"
    );
}

#[test]
fn the_legacy_alias_is_used_when_the_role_named_one_is_unset() {
    let resolved = resolve_alias(None, Some("http://legacy"));
    assert_eq!(
        resolved.value.as_deref(),
        Some("http://legacy"),
        "an existing setup must keep working untouched — that is the whole \
         point of keeping the alias"
    );
    assert!(
        !resolved.conflicting,
        "using the alias as intended is not a conflict"
    );
}

#[test]
fn the_same_value_under_both_names_is_silent() {
    let resolved = resolve_alias(Some("http://same"), Some("http://same"));
    assert_eq!(resolved.value.as_deref(), Some("http://same"));
    assert!(
        !resolved.conflicting,
        "a migration that sets both to the same thing is correct, not suspicious \
         — warning about it would train the operator to ignore the warning"
    );
}

#[test]
fn neither_name_set_resolves_to_nothing() {
    let resolved = resolve_alias(None, None);
    assert!(resolved.value.is_none());
    assert!(!resolved.conflicting);
}

#[test]
fn conflicts_are_reported_in_one_notice_naming_both_variables() {
    assert!(
        alias_conflict_notice(&[]).is_none(),
        "no conflict must produce no output at all"
    );
    let notice = alias_conflict_notice(&[
        ("VELESDB_MEMORY_EMBEDDER_URL", "VELESDB_MEMORY_OLLAMA_URL"),
        (
            "VELESDB_MEMORY_EMBEDDER_MODEL",
            "VELESDB_MEMORY_OLLAMA_MODEL",
        ),
    ])
    .expect("two conflicts must produce a notice");
    for name in [
        "VELESDB_MEMORY_EMBEDDER_URL",
        "VELESDB_MEMORY_OLLAMA_URL",
        "VELESDB_MEMORY_EMBEDDER_MODEL",
        "VELESDB_MEMORY_OLLAMA_MODEL",
    ] {
        assert!(
            notice.contains(name),
            "the notice must name {name} so the operator can find it, got: {notice}"
        );
    }
    assert_eq!(
        notice.lines().count(),
        1,
        "ONE notice, however many variables disagree: a warning per variable is \
         how a startup log becomes noise, got: {notice}"
    );
}

// --- B1: a credential never lives in the file ------------------------------

#[test]
fn an_api_token_in_the_file_is_refused_for_both_roles() {
    // The rule, and the reason: a token at rest in a TOML is a secret one
    // `git add .` away from a public history, and this repo's pre-commit
    // secret guard cannot police a file it has never seen. Tokens are read
    // from the environment ONLY. `deny_unknown_fields` is what makes that
    // structural rather than a convention — the field does not exist, so it
    // cannot be silently accepted and ignored either, which would be worse:
    // the operator would believe the daemon authenticates when it does not.
    for role in ["embedder", "extractor"] {
        let (_dir, path) = config_file(&format!("[{role}]\napi_token = \"sk-secret\"\n"));
        let Err(err) = load(&path) else {
            panic!("[{role}] api_token must be refused, not accepted");
        };
        let ConfigError::Parse { message, .. } = &err else {
            panic!("[{role}]: expected a parse refusal, got {err:?}");
        };
        assert!(
            message.contains("api_token"),
            "[{role}]: the refusal must name the offending key, got: {message}"
        );
    }
}

#[test]
fn refusing_an_api_token_says_where_the_token_belongs() {
    // "Unknown field" alone leaves an operator with a working token and
    // nowhere to put it; the whole point of failing at boot is to hand back
    // the next step. The hint names the role's own variable, so it is
    // actionable without a trip to the README.
    let (_dir, path) = config_file("[extractor]\napi_token = \"sk-secret\"\n");
    let err = load(&path).expect_err("api_token must be refused");
    let rendered = err.to_string();
    assert!(
        rendered.contains("VELESDB_MEMORY_EXTRACTOR_API_TOKEN"),
        "the refusal must name the variable to use instead, got: {rendered}"
    );
    assert!(
        !rendered.contains("sk-secret"),
        "the refusal must never echo the credential back, got: {rendered}"
    );
}

#[test]
fn a_typo_is_rejected_rather_than_silently_dropped() {
    let (_dir, path) = config_file("[embedder]\nmdoel = \"bge-m3\"\n");
    let err = load(&path).expect_err("unknown key must fail");
    assert!(
        matches!(err, ConfigError::Parse { .. }),
        "got {err:?} — a typo'd key must not be silently ignored"
    );
    // POSITIVE CONTROL for the redaction above. `toml` quotes the offending
    // source line back, which is a gift here — the operator sees exactly what
    // to fix — and a credential leak on the `api_token` line. If this
    // assertion ever fails, `toml` stopped echoing the source and
    // `describe_parse_failure`'s redaction has become a guard with nothing
    // left to catch: check before deleting it, do not assume.
    assert!(
        err.to_string().contains("bge-m3"),
        "the parser is expected to quote the offending line verbatim — that is \
         WHY the api_token failure has to be rewritten. Got: {err}"
    );
}

#[test]
fn malformed_toml_is_an_error_not_a_silent_default() {
    let (_dir, path) = config_file("this is not toml {{{");
    assert!(matches!(
        load(&path).expect_err("must fail"),
        ConfigError::Parse { .. }
    ));
}

#[test]
fn a_missing_file_is_reported_as_unreadable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let err = load(&dir.path().join("absent.toml")).expect_err("must fail");
    assert!(matches!(err, ConfigError::Read { .. }));
}

#[test]
fn ingest_roots_are_joined_into_the_platform_list_syntax() {
    let (_dir, path) = config_file("[context]\ningest_roots = [\"/tmp\", \"/var\"]\n");
    let loaded = load(&path).expect("valid config");
    let joined = &loaded.values["VELESDB_MEMORY_INGEST_ROOTS"];
    let parts: Vec<_> = std::env::split_paths(joined).collect();
    assert_eq!(
        parts.len(),
        2,
        "both roots survive the round trip: {joined}"
    );
}

/// The precedence rule, which is the entire contract: the environment was set
/// by whoever launched the process and outranks a file on disk.
#[test]
fn apply_never_overwrites_an_existing_variable() {
    let key = "VELESDB_MEMORY_TEST_PRECEDENCE";
    std::env::set_var(key, "from-env");
    let mut values = BTreeMap::new();
    values.insert(key.to_string(), "from-file".to_string());

    let applied = apply(&values);

    assert!(applied.is_empty(), "an already-set variable is left alone");
    assert_eq!(std::env::var(key).as_deref(), Ok("from-env"));
    std::env::remove_var(key);
}

#[test]
fn apply_sets_a_variable_that_is_absent() {
    let key = "VELESDB_MEMORY_TEST_ABSENT";
    std::env::remove_var(key);
    let mut values = BTreeMap::new();
    values.insert(key.to_string(), "from-file".to_string());

    let applied = apply(&values);

    assert_eq!(applied, vec![key.to_string()]);
    assert_eq!(std::env::var(key).as_deref(), Ok("from-file"));
    std::env::remove_var(key);
}

#[test]
fn an_explicit_path_wins_over_every_lookup() {
    let resolved = resolve_path(Some("/explicit/velesdb-memory.toml"), None);
    assert_eq!(
        resolved,
        Some(PathBuf::from("/explicit/velesdb-memory.toml"))
    );
}

#[test]
fn the_store_directory_is_searched_before_the_working_directory() {
    let (dir, path) = config_file("quiet = true\n");
    let resolved = resolve_path(None, Some(dir.path()));
    assert_eq!(resolved, Some(path));
}

#[test]
fn no_file_anywhere_resolves_to_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert_eq!(resolve_path(None, Some(dir.path())), None);
}
