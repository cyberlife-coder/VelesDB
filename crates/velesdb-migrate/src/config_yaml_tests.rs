//! The YAML loader's contract: the documented configs keep parsing to the
//! same values, and the spellings that were strings stay strings.

use super::*;
use std::path::Path;

fn crate_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn source_type(config: &MigrationConfig) -> &'static str {
    match config.source {
        SourceConfig::Supabase(_) => "supabase",
        SourceConfig::Qdrant(_) => "qdrant",
        SourceConfig::Pinecone(_) => "pinecone",
        SourceConfig::Weaviate(_) => "weaviate",
        SourceConfig::Milvus(_) => "milvus",
        SourceConfig::ChromaDB(_) => "chromadb",
        SourceConfig::JsonFile(_) => "json_file",
        SourceConfig::CsvFile(_) => "csv_file",
        SourceConfig::Elasticsearch(_) => "elasticsearch",
        SourceConfig::Redis(_) => "redis",
    }
}

/// `(file, destination.dimension)` for every shipped example the loader accepts.
const LOADABLE_EXAMPLES: &[(&str, usize)] = &[
    ("chromadb-migration.yaml", 768),
    ("milvus-migration.yaml", 768),
    ("pinecone-migration.yaml", 1536),
    ("qdrant-migration.yaml", 768),
    ("supabase-migration.yaml", 768),
    ("weaviate-migration.yaml", 768),
];

#[test]
fn test_shipped_examples_load_through_from_file() {
    for (file, dimension) in LOADABLE_EXAMPLES {
        let path = crate_root().join("examples").join(file);
        let config =
            MigrationConfig::from_file(&path).unwrap_or_else(|e| panic!("{file} must load: {e}"));
        let stem = file.trim_end_matches("-migration.yaml");
        assert_eq!(source_type(&config), stem, "{file}: source type");
        assert_eq!(
            config.destination.dimension, *dimension,
            "{file}: dimension"
        );
        assert!(config.validate().is_ok(), "{file}: must validate");
    }
}

/// Every `source:` YAML block of the sources guide is a config an operator
/// copies; each must load. The count guards against the extraction matching
/// nothing and the loop passing vacuously.
#[test]
fn test_every_yaml_block_of_the_sources_guide_loads() {
    let guide = crate_root().join("../../docs/guides/MIGRATE_SOURCES.md");
    let text = std::fs::read_to_string(&guide).expect("test: sources guide is readable");
    let blocks: Vec<&str> = text
        .split("```yaml\n")
        .skip(1)
        .filter_map(|rest| rest.split("```").next())
        .filter(|block| block.starts_with("source:"))
        .collect();
    assert!(blocks.len() >= 10, "found only {} blocks", blocks.len());
    for block in blocks {
        MigrationConfig::parse_yaml(block)
            .unwrap_or_else(|e| panic!("guide block must load: {e}\n{block}"));
    }
}

const MINIMAL_DESTINATION: &str = "destination: {path: ./d, collection: c, dimension: 3}\n";

fn qdrant_with(extra: &str) -> String {
    format!("source: {{type: qdrant, url: http://x, collection: c{extra}}}\n{MINIMAL_DESTINATION}")
}

/// YAML 1.2 booleans: `yes`/`no`/`on`/`off` are names, not flags. A parser
/// that read them as booleans would reject a source named `on`.
#[test]
fn test_yaml_1_1_boolean_spellings_stay_strings() {
    for word in ["yes", "Yes", "no", "on", "off", "y", "n"] {
        let config = MigrationConfig::parse_yaml(&qdrant_with(&format!(", api_key: {word}")))
            .unwrap_or_else(|e| panic!("`api_key: {word}` must load: {e}"));
        let SourceConfig::Qdrant(qdrant) = config.source else {
            panic!("test: qdrant source");
        };
        assert_eq!(qdrant.api_key.as_deref(), Some(word));
    }
    let yaml = format!("{}options: {{dry_run: yes}}\n", qdrant_with(""));
    assert!(MigrationConfig::parse_yaml(&yaml).is_err());
}

#[test]
fn test_value_forms_in_common_use_load() {
    let yaml = "\
# comment
source:
  type: supabase
  url: 'http://x'
  api_key: \"a: b # c\"
  table: t
  payload_columns: [a, \"b\", 'c']
destination:
  path: ./d
  collection: c
  dimension: 0x10
  metric: dot_product
  storage_mode: rabitq
options:
  batch_size: 10
  dry_run: true
relations: []
";
    let config = MigrationConfig::parse_yaml(yaml).expect("test: valid config");
    let SourceConfig::Supabase(supabase) = config.source else {
        panic!("test: supabase source");
    };
    assert_eq!(supabase.api_key, "a: b # c");
    assert_eq!(supabase.payload_columns, ["a", "b", "c"]);
    assert_eq!(config.destination.dimension, 16);
    assert!(matches!(config.destination.metric, DistanceMetric::Dot));
    assert!(matches!(
        config.destination.storage_mode,
        StorageMode::RaBitQ
    ));
    assert_eq!(config.options.batch_size, 10);
    assert!(config.options.dry_run);
}

#[test]
fn test_crlf_line_endings_load() {
    let yaml = qdrant_with("").replace('\n', "\r\n");
    assert!(MigrationConfig::parse_yaml(&yaml).is_ok());
}

/// The README documents this: a misspelled field is ignored, not rejected.
#[test]
fn test_unknown_keys_are_ignored() {
    let yaml = format!("{}extra_top_level: 1\n", qdrant_with(", bogus: 1"));
    assert!(MigrationConfig::parse_yaml(&yaml).is_ok());
}

/// Pinecone's `environment` field is gone (8.0.0); a config written for an
/// earlier release still sets it and must keep loading.
#[test]
fn test_a_pinecone_config_that_still_sets_environment_loads() {
    let yaml = format!(
        "source: {{type: pinecone, api_key: k, environment: us-east-1-aws, index: i}}\n\
         {MINIMAL_DESTINATION}"
    );
    let config = MigrationConfig::parse_yaml(&yaml).expect("test: pre-8.0 pinecone config");
    let SourceConfig::Pinecone(pinecone) = config.source else {
        panic!("test: pinecone source");
    };
    assert_eq!(pinecone.index, "i");
}

#[test]
fn test_invalid_documents_are_rejected_with_a_yaml_error() {
    let duplicate = "source: {type: qdrant, url: u, collection: c}\n\
                     destination: {path: p, collection: c, dimension: 3, dimension: 4}\n";
    let two_documents = format!("{}---\nother: 1\n", qdrant_with(""));
    let cases = [
        ("empty", ""),
        ("unclosed flow", "source: [\n"),
        ("duplicate key", duplicate),
        ("two documents", two_documents.as_str()),
        ("unknown source type", "source: {type: nosuch}\n"),
        ("missing field", "source: {type: qdrant, url: u}\n"),
    ];
    for (name, yaml) in cases {
        let err = MigrationConfig::parse_yaml(yaml).expect_err(name);
        assert!(
            matches!(err, crate::error::Error::Yaml(_)),
            "{name}: {err:?}"
        );
    }
}

#[test]
fn test_yaml_error_message_locates_the_problem() {
    let err = MigrationConfig::parse_yaml("source: {type: nosuch}\n").unwrap_err();
    let message = err.to_string();
    assert!(message.starts_with("YAML error: "), "{message}");
    assert!(message.contains("nosuch"), "{message}");
    assert!(message.contains("line 1"), "{message}");
    assert!(
        !message.contains('\n'),
        "one line, not a snippet: {message}"
    );
}

#[test]
fn test_from_file_reports_a_missing_file_as_io_not_yaml() {
    let err =
        MigrationConfig::from_file(Path::new("/nonexistent/velesdb/migration.yaml")).unwrap_err();
    assert!(matches!(err, crate::error::Error::Io(_)), "{err:?}");
}
