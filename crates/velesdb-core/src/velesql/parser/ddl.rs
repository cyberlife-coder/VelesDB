//! DDL statement parsing (CREATE/DROP COLLECTION, CREATE/DROP INDEX, ANALYZE, TRUNCATE, ALTER).

use super::{extract_identifier, Rule};
use crate::velesql::ast::{
    AlterCollectionStatement, AnalyzeStatement, CreateCollectionStatement, CreateIndexStatement,
    DdlStatement, DropCollectionStatement, DropIndexStatement, Query, TruncateStatement,
};
use crate::velesql::error::ParseError;
use crate::velesql::Parser;

use super::ddl_helpers::{
    build_collection_kind, parse_create_body, parse_create_options, CreateSuffix,
};

impl Parser {
    /// Parses a `CREATE COLLECTION` statement.
    ///
    /// Grammar:
    /// ```text
    /// CREATE [GRAPH|METADATA] COLLECTION name [(options)] [suffix]
    /// ```
    pub(crate) fn parse_create_collection_stmt(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Query, ParseError> {
        let mut kind_kw: Option<String> = None;
        let mut name: Option<String> = None;
        let mut options: Vec<(String, String)> = Vec::new();
        let mut suffix: Option<CreateSuffix> = None;

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::collection_kind_kw => {
                    kind_kw = Some(inner.as_str().to_ascii_uppercase());
                }
                Rule::identifier if name.is_none() => {
                    name = Some(extract_identifier(&inner));
                }
                Rule::create_body => {
                    let (opts, sfx) = parse_create_body(inner)?;
                    options = opts;
                    suffix = sfx;
                }
                _ => {}
            }
        }

        let name = name.ok_or_else(|| {
            ParseError::syntax(0, "", "CREATE COLLECTION requires a collection name")
        })?;
        let kind = build_collection_kind(kind_kw.as_deref(), &options, suffix)?;

        Ok(Query::new_ddl(DdlStatement::CreateCollection(
            CreateCollectionStatement { name, kind },
        )))
    }

    /// Parses a `DROP COLLECTION` statement.
    ///
    /// Grammar:
    /// ```text
    /// DROP COLLECTION [IF EXISTS] name
    /// ```
    pub(crate) fn parse_drop_collection_stmt(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Query, ParseError> {
        let mut if_exists = false;
        let mut name: Option<String> = None;

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::if_exists_clause => if_exists = true,
                Rule::identifier if name.is_none() => {
                    name = Some(extract_identifier(&inner));
                }
                _ => {}
            }
        }

        let name = name.ok_or_else(|| {
            ParseError::syntax(0, "", "DROP COLLECTION requires a collection name")
        })?;

        Ok(Query::new_ddl(DdlStatement::DropCollection(
            DropCollectionStatement { name, if_exists },
        )))
    }

    /// Parses a `CREATE INDEX ON collection (field)` statement.
    ///
    /// Grammar:
    /// ```text
    /// CREATE INDEX ON identifier ( identifier )
    /// ```
    pub(crate) fn parse_create_index_stmt(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Query, ParseError> {
        let (collection, field) = extract_index_identifiers(pair, "CREATE INDEX")?;
        Ok(Query::new_ddl(DdlStatement::CreateIndex(
            CreateIndexStatement { collection, field },
        )))
    }

    /// Parses a `DROP INDEX ON collection (field)` statement.
    ///
    /// Grammar:
    /// ```text
    /// DROP INDEX ON identifier ( identifier )
    /// ```
    pub(crate) fn parse_drop_index_stmt(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Query, ParseError> {
        let (collection, field) = extract_index_identifiers(pair, "DROP INDEX")?;
        Ok(Query::new_ddl(DdlStatement::DropIndex(
            DropIndexStatement { collection, field },
        )))
    }

    /// Parses an `ANALYZE [COLLECTION] name` statement.
    ///
    /// Grammar:
    /// ```text
    /// ANALYZE [COLLECTION] identifier
    /// ```
    pub(crate) fn parse_analyze_stmt(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Query, ParseError> {
        let collection = extract_single_identifier(pair, "ANALYZE")?;
        Ok(Query::new_ddl(DdlStatement::Analyze(AnalyzeStatement {
            collection,
        })))
    }

    /// Parses a `TRUNCATE [COLLECTION] name` statement.
    ///
    /// Grammar:
    /// ```text
    /// TRUNCATE [COLLECTION] identifier
    /// ```
    pub(crate) fn parse_truncate_stmt(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Query, ParseError> {
        let collection = extract_single_identifier(pair, "TRUNCATE")?;
        Ok(Query::new_ddl(DdlStatement::Truncate(TruncateStatement {
            collection,
        })))
    }

    /// Parses an `ALTER COLLECTION name SET (options)` statement.
    ///
    /// Grammar:
    /// ```text
    /// ALTER COLLECTION identifier SET ( create_option_list )
    /// ```
    pub(crate) fn parse_alter_collection_stmt(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Query, ParseError> {
        let mut name: Option<String> = None;
        let mut options: Vec<(String, String)> = Vec::new();

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::identifier if name.is_none() => {
                    name = Some(extract_identifier(&inner));
                }
                Rule::create_option_list => {
                    options = parse_create_options(inner)?;
                }
                _ => {}
            }
        }

        let collection = name.ok_or_else(|| {
            ParseError::syntax(0, "", "ALTER COLLECTION requires a collection name")
        })?;

        Ok(Query::new_ddl(DdlStatement::AlterCollection(
            AlterCollectionStatement {
                collection,
                options,
            },
        )))
    }
}

/// Extracts the two identifiers (collection, field) from a CREATE/DROP INDEX pair.
///
/// Both `create_index_stmt` and `drop_index_stmt` share the same structure:
/// `keyword ON identifier ( identifier )`, so this helper avoids duplication.
fn extract_index_identifiers(
    pair: pest::iterators::Pair<Rule>,
    context: &str,
) -> Result<(String, String), ParseError> {
    let mut idents = pair
        .into_inner()
        .filter(|p| p.as_rule() == Rule::identifier)
        .map(|p| extract_identifier(&p));

    let collection = idents.next().ok_or_else(|| {
        ParseError::syntax(0, "", format!("{context} requires a collection name"))
    })?;
    let field = idents
        .next()
        .ok_or_else(|| ParseError::syntax(0, "", format!("{context} requires a field name")))?;

    Ok((collection, field))
}

/// Extracts a single identifier from a statement pair (ANALYZE, TRUNCATE).
///
/// Both `analyze_stmt` and `truncate_stmt` have the same structure:
/// `keyword [COLLECTION] identifier`, so this helper avoids duplication.
fn extract_single_identifier(
    pair: pest::iterators::Pair<Rule>,
    context: &str,
) -> Result<String, ParseError> {
    pair.into_inner()
        .find(|p| p.as_rule() == Rule::identifier)
        .map(|p| extract_identifier(&p))
        .ok_or_else(|| ParseError::syntax(0, "", format!("{context} requires a collection name")))
}
