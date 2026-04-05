//! Private helpers for DDL statement parsing.
//!
//! Contains option lookup, schema definition parsing, and collection kind
//! construction logic extracted from `ddl.rs`.

use super::{extract_identifier, Rule};
use crate::velesql::ast::{
    CreateCollectionKind, GraphCollectionParams, GraphSchemaMode, SchemaDefinition,
    VectorCollectionParams,
};
use crate::velesql::error::ParseError;
use crate::velesql::Parser;

/// Internal representation of `create_suffix` before final AST construction.
pub(super) enum CreateSuffix {
    Schemaless,
    TypedSchema(Vec<SchemaDefinition>),
    With(Vec<(String, String)>),
}

/// Parsed body of a CREATE COLLECTION statement (options + optional suffix).
pub(super) type CreateBody = (Vec<(String, String)>, Option<CreateSuffix>);

/// Parses the `create_body` rule: `(options) [suffix]`.
pub(super) fn parse_create_body(
    pair: pest::iterators::Pair<Rule>,
) -> Result<CreateBody, ParseError> {
    let mut options = Vec::new();
    let mut suffix = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::create_option_list => options = parse_create_options(inner)?,
            Rule::create_suffix => suffix = Some(parse_create_suffix(inner)?),
            _ => {}
        }
    }

    Ok((options, suffix))
}

/// Extracts key=value pairs from `create_option_list`.
pub(super) fn parse_create_options(
    pair: pest::iterators::Pair<Rule>,
) -> Result<Vec<(String, String)>, ParseError> {
    super::helpers::extract_key_value_list(pair, Rule::create_option, |p| {
        Ok(parse_single_create_option(p))
    })
}

/// Parses a single `create_option`: `identifier = create_option_value`.
fn parse_single_create_option(pair: pest::iterators::Pair<Rule>) -> (String, String) {
    let mut key = String::new();
    let mut value = String::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::identifier if key.is_empty() => {
                key = extract_identifier(&inner).to_ascii_lowercase();
            }
            Rule::create_option_value => {
                value = extract_option_value(inner);
            }
            _ => {}
        }
    }

    (key, value)
}

/// Extracts the string representation of a `create_option_value`.
fn extract_option_value(pair: pest::iterators::Pair<Rule>) -> String {
    let inner = pair.into_inner().next();
    match inner {
        Some(p) => match p.as_rule() {
            Rule::string => crate::velesql::parser::helpers::unescape_string_literal(p.as_str()),
            Rule::identifier => extract_identifier(&p),
            _ => p.as_str().to_string(),
        },
        None => String::new(),
    }
}

/// Dispatches `create_suffix` to SCHEMALESS, WITH SCHEMA, or WITH.
fn parse_create_suffix(pair: pest::iterators::Pair<Rule>) -> Result<CreateSuffix, ParseError> {
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| ParseError::syntax(0, "", "Expected CREATE suffix"))?;

    match inner.as_rule() {
        Rule::schemaless_clause => Ok(CreateSuffix::Schemaless),
        Rule::with_schema_clause => {
            let defs = parse_schema_definitions(inner)?;
            Ok(CreateSuffix::TypedSchema(defs))
        }
        Rule::with_clause => {
            let with = Parser::parse_with_clause(inner)?;
            let opts = with
                .options
                .into_iter()
                .map(|o| (o.key, with_value_to_string(o.value)))
                .collect();
            Ok(CreateSuffix::With(opts))
        }
        _ => Err(ParseError::syntax(
            0,
            inner.as_str(),
            "Unknown CREATE suffix",
        )),
    }
}

/// Parses `with_schema_clause` into a list of `SchemaDefinition`.
fn parse_schema_definitions(
    pair: pest::iterators::Pair<Rule>,
) -> Result<Vec<SchemaDefinition>, ParseError> {
    pair.into_inner()
        .find(|p| p.as_rule() == Rule::schema_def_list)
        .map_or_else(
            || Ok(Vec::new()),
            |list| {
                list.into_inner()
                    .filter(|p| p.as_rule() == Rule::schema_def)
                    .map(parse_single_schema_def)
                    .collect()
            },
        )
}

/// Parses a single `schema_def` (NODE or EDGE type definition).
fn parse_single_schema_def(
    pair: pest::iterators::Pair<Rule>,
) -> Result<SchemaDefinition, ParseError> {
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| ParseError::syntax(0, "", "Expected schema definition"))?;

    match inner.as_rule() {
        Rule::node_type_def => Ok(parse_node_type_def(inner)),
        Rule::edge_type_def => parse_edge_type_def(inner),
        _ => Err(ParseError::syntax(
            0,
            inner.as_str(),
            "Expected NODE or EDGE definition",
        )),
    }
}

/// Parses `NODE TypeName (prop1: Type, ...)`.
fn parse_node_type_def(pair: pest::iterators::Pair<Rule>) -> SchemaDefinition {
    let mut name = String::new();
    let mut properties = Vec::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::identifier if name.is_empty() => {
                name = extract_identifier(&inner);
            }
            Rule::property_def_list => {
                properties = parse_property_def_list(inner);
            }
            _ => {}
        }
    }

    SchemaDefinition::Node { name, properties }
}

/// Parses `EDGE TypeName FROM Source TO Target`.
fn parse_edge_type_def(pair: pest::iterators::Pair<Rule>) -> Result<SchemaDefinition, ParseError> {
    let mut idents = pair
        .into_inner()
        .filter(|p| p.as_rule() == Rule::identifier)
        .map(|p| extract_identifier(&p));

    let name = idents
        .next()
        .ok_or_else(|| ParseError::syntax(0, "", "EDGE definition requires a name"))?;
    let from_type = idents
        .next()
        .ok_or_else(|| ParseError::syntax(0, "", "EDGE definition requires FROM type"))?;
    let to_type = idents
        .next()
        .ok_or_else(|| ParseError::syntax(0, "", "EDGE definition requires TO type"))?;

    Ok(SchemaDefinition::Edge {
        name,
        from_type,
        to_type,
    })
}

/// Parses `property_def_list` into `(name, type_name)` pairs.
fn parse_property_def_list(pair: pest::iterators::Pair<Rule>) -> Vec<(String, String)> {
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::property_def)
        .map(parse_single_property_def)
        .collect()
}

/// Parses a single `property_def`: `identifier : type_name`.
fn parse_single_property_def(pair: pest::iterators::Pair<Rule>) -> (String, String) {
    let mut name = String::new();
    let mut type_name = String::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::identifier if name.is_empty() => {
                name = extract_identifier(&inner);
            }
            Rule::type_name => {
                type_name = inner.as_str().to_ascii_uppercase();
            }
            _ => {}
        }
    }

    (name, type_name)
}

/// Builds the appropriate `CreateCollectionKind` from parsed components.
pub(super) fn build_collection_kind(
    kind_kw: Option<&str>,
    options: &[(String, String)],
    suffix: Option<CreateSuffix>,
) -> Result<CreateCollectionKind, ParseError> {
    match kind_kw {
        Some("METADATA") => Ok(CreateCollectionKind::Metadata),
        Some("GRAPH") => build_graph_params(options, suffix),
        _ => build_vector_params(options, suffix),
    }
}

/// Constructs `CreateCollectionKind::Vector` from parsed options.
fn build_vector_params(
    options: &[(String, String)],
    suffix: Option<CreateSuffix>,
) -> Result<CreateCollectionKind, ParseError> {
    let dimension = lookup_required_usize(options, "dimension", "Vector collection")?;
    let metric = lookup_optional_str(options, "metric").unwrap_or_else(|| "cosine".to_string());
    validate_metric(&metric)?;
    let mut storage = lookup_optional_str(options, "storage");

    let (m, ef_construction) = extract_with_suffix_hnsw(options, suffix, &mut storage);

    Ok(CreateCollectionKind::Vector(VectorCollectionParams {
        dimension,
        metric,
        storage,
        m,
        ef_construction,
    }))
}

/// Constructs `CreateCollectionKind::Graph` from parsed options.
fn build_graph_params(
    options: &[(String, String)],
    suffix: Option<CreateSuffix>,
) -> Result<CreateCollectionKind, ParseError> {
    let dimension = lookup_optional_usize(options, "dimension");
    let metric = lookup_optional_str(options, "metric");
    if let Some(ref m) = metric {
        validate_metric(m)?;
    }
    let schema_mode = match suffix {
        Some(CreateSuffix::TypedSchema(defs)) => GraphSchemaMode::Typed(defs),
        Some(CreateSuffix::Schemaless | CreateSuffix::With(_)) | None => {
            GraphSchemaMode::Schemaless
        }
    };

    Ok(CreateCollectionKind::Graph(GraphCollectionParams {
        dimension,
        metric,
        schema_mode,
    }))
}

/// Extracts HNSW parameters and storage from body options + WITH suffix.
fn extract_with_suffix_hnsw(
    options: &[(String, String)],
    suffix: Option<CreateSuffix>,
    storage: &mut Option<String>,
) -> (Option<usize>, Option<usize>) {
    let mut m = lookup_optional_usize(options, "m");
    let mut ef_construction = lookup_optional_usize(options, "ef_construction");

    if let Some(CreateSuffix::With(with_opts)) = suffix {
        if storage.is_none() {
            *storage = lookup_optional_str(&with_opts, "storage");
        }
        if m.is_none() {
            m = lookup_optional_usize(&with_opts, "m");
        }
        if ef_construction.is_none() {
            ef_construction = lookup_optional_usize(&with_opts, "ef_construction");
        }
    }

    (m, ef_construction)
}

// ---------------------------------------------------------------------------
// Option lookup helpers
// ---------------------------------------------------------------------------

/// Finds a required string option by key.
fn lookup_required_str(
    options: &[(String, String)],
    key: &str,
    context: &str,
) -> Result<String, ParseError> {
    options
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
        .ok_or_else(|| ParseError::syntax(0, "", format!("{context} requires '{key}' option")))
}

/// Finds a required `usize` option by key.
fn lookup_required_usize(
    options: &[(String, String)],
    key: &str,
    context: &str,
) -> Result<usize, ParseError> {
    let raw = lookup_required_str(options, key, context)?;
    raw.parse::<usize>()
        .map_err(|_| ParseError::syntax(0, &raw, format!("'{key}' must be a positive integer")))
}

/// Finds an optional string option by key.
fn lookup_optional_str(options: &[(String, String)], key: &str) -> Option<String> {
    options
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
}

/// Finds an optional `usize` option by key (silently ignores parse failures).
fn lookup_optional_usize(options: &[(String, String)], key: &str) -> Option<usize> {
    lookup_optional_str(options, key).and_then(|v| v.parse::<usize>().ok())
}

/// Validates that a metric string is a recognized `DistanceMetric` alias.
fn validate_metric(metric: &str) -> Result<(), ParseError> {
    match metric.to_lowercase().as_str() {
        "cosine" | "euclidean" | "l2" | "dot" | "dotproduct" | "inner" | "ip" | "hamming"
        | "jaccard" => Ok(()),
        _ => Err(ParseError::syntax(
            0,
            metric,
            format!(
                "Unknown metric '{metric}'. Supported: cosine, euclidean, \
                 l2, dot, dotproduct, inner, ip, hamming, jaccard"
            ),
        )),
    }
}

/// Converts a `WithValue` to its string representation for storage in options.
pub(super) fn with_value_to_string(value: crate::velesql::ast::WithValue) -> String {
    use crate::velesql::ast::WithValue;
    match value {
        WithValue::String(s) | WithValue::Identifier(s) => s,
        WithValue::Integer(i) => i.to_string(),
        WithValue::Float(f) => f.to_string(),
        WithValue::Boolean(b) => b.to_string(),
    }
}
