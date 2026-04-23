//! Python bindings for VelesQL query language.
//!
//! Provides parser access and query introspection for Python users.
//!
//! # Example
//!
//! ```python
//! from velesdb import VelesQL
//!
//! # Parse a query
//! parsed = VelesQL.parse("SELECT * FROM docs WHERE category = 'tech' LIMIT 10")
//! assert parsed.is_valid()
//! assert parsed.collection_name == "docs"
//! ```

// Exceptions created via pyo3::create_exception! macro
use pyo3::prelude::*;
use velesdb_core::velesql::{Parser as CoreParser, Query as CoreQuery};

pub use super::velesql_helpers::register_velesql_module;

// VelesQL syntax error exception.
pyo3::create_exception!(velesdb, VelesQLSyntaxError, pyo3::exceptions::PyException);

// VelesQL parameter error exception.
pyo3::create_exception!(
    velesdb,
    VelesQLParameterError,
    pyo3::exceptions::PyException
);

/// VelesQL query parser.
///
/// Example:
///     >>> from velesdb import VelesQL
///     >>> parsed = VelesQL.parse("SELECT * FROM docs LIMIT 10")
///     >>> print(parsed.collection_name)
///     'docs'
#[pyclass(frozen)]
pub struct VelesQL;

#[pymethods]
impl VelesQL {
    /// Parse a VelesQL query string.
    ///
    /// Args:
    ///     query: VelesQL query string to parse
    ///
    /// Returns:
    ///     ParsedStatement: Parsed query object for introspection
    ///
    /// Raises:
    ///     VelesQLSyntaxError: If the query has syntax errors
    ///
    /// Example:
    ///     >>> parsed = VelesQL.parse("SELECT * FROM documents WHERE category = 'tech'")
    ///     >>> assert parsed.is_valid()
    ///     >>> assert parsed.collection_name == "documents"
    #[staticmethod]
    fn parse(query: &str) -> PyResult<ParsedStatement> {
        CoreParser::parse(query)
            .map(|q| ParsedStatement { inner: q })
            .map_err(|e| {
                VelesQLSyntaxError::new_err(super::velesql_helpers::format_parse_error(&e))
            })
    }

    /// Validate a VelesQL query without full parsing.
    ///
    /// This is faster than parse() when you only need to check validity.
    ///
    /// Args:
    ///     query: VelesQL query string to validate
    ///
    /// Returns:
    ///     bool: True if the query is syntactically valid
    ///
    /// Example:
    ///     >>> VelesQL.is_valid("SELECT * FROM docs")
    ///     True
    ///     >>> VelesQL.is_valid("SELEC * FROM docs")
    ///     False
    #[staticmethod]
    fn is_valid(query: &str) -> bool {
        CoreParser::parse(query).is_ok()
    }
}

/// A parsed VelesQL statement.
///
/// Provides introspection into the query structure.
///
/// Example:
///     >>> parsed = VelesQL.parse("SELECT id, name FROM users WHERE active = true ORDER BY name")
///     >>> print(parsed.columns)
///     ['id', 'name']
///     >>> print(parsed.has_where_clause())
///     True
#[pyclass(frozen)]
pub struct ParsedStatement {
    pub(crate) inner: CoreQuery,
}

#[pymethods]
impl ParsedStatement {
    /// Check if the query is valid (always True for successfully parsed queries).
    ///
    /// Returns:
    ///     bool: True (always, since invalid queries raise exceptions during parsing)
    fn is_valid(&self) -> bool {
        true
    }

    /// Check if this is a SELECT query.
    ///
    /// Returns:
    ///     bool: True if this is a SELECT query
    fn is_select(&self) -> bool {
        self.inner.is_select_query()
    }

    /// Check if this is a MATCH (graph) query.
    ///
    /// Returns:
    ///     bool: True if this is a MATCH query
    fn is_match(&self) -> bool {
        self.inner.is_match_query()
    }

    /// Check if this is a DDL statement (CREATE/DROP COLLECTION).
    ///
    /// Returns:
    ///     bool: True if this is a DDL statement
    fn is_ddl(&self) -> bool {
        self.inner.is_ddl_query()
    }

    /// Check if this is a DML statement (INSERT/UPDATE/DELETE).
    ///
    /// Returns:
    ///     bool: True if this is a DML statement
    fn is_dml(&self) -> bool {
        self.inner.is_dml_query()
    }

    /// Check if this is a DELETE statement.
    ///
    /// Returns:
    ///     bool: True if this is a DELETE or DELETE EDGE statement
    fn is_delete(&self) -> bool {
        matches!(
            &self.inner.dml,
            Some(velesdb_core::velesql::DmlStatement::Delete(_))
                | Some(velesdb_core::velesql::DmlStatement::DeleteEdge(_))
        )
    }

    /// Check if this is an INSERT EDGE statement.
    ///
    /// Returns:
    ///     bool: True if this is an INSERT EDGE statement
    fn is_insert_edge(&self) -> bool {
        matches!(
            &self.inner.dml,
            Some(velesdb_core::velesql::DmlStatement::InsertEdge(_))
        )
    }

    /// Get the collection name from the FROM clause, DDL, or DML statement.
    ///
    /// Returns:
    ///     str or None: Collection name, or None for MATCH queries
    #[getter]
    fn collection_name(&self) -> Option<String> {
        if let Some(ddl) = &self.inner.ddl {
            return Some(match ddl {
                velesdb_core::velesql::DdlStatement::CreateCollection(s) => s.name.clone(),
                velesdb_core::velesql::DdlStatement::DropCollection(s) => s.name.clone(),
                velesdb_core::velesql::DdlStatement::CreateIndex(s) => s.collection.clone(),
                velesdb_core::velesql::DdlStatement::DropIndex(s) => s.collection.clone(),
                velesdb_core::velesql::DdlStatement::Analyze(s) => s.collection.clone(),
                velesdb_core::velesql::DdlStatement::Truncate(s) => s.collection.clone(),
                velesdb_core::velesql::DdlStatement::AlterCollection(s) => s.collection.clone(),
                _ => return None,
            });
        }
        if let Some(name) = Self::dml_collection_name(&self.inner) {
            return Some(name);
        }
        let from = &self.inner.select.from;
        if from.is_empty() {
            None
        } else {
            Some(from.clone())
        }
    }

    /// Get the collection alias if present (for self-joins).
    ///
    /// Returns:
    ///     str or None: First collection alias, or None if not aliased
    #[getter]
    fn table_alias(&self) -> Option<String> {
        self.inner.select.from_alias.first().cloned()
    }

    /// Get all aliases visible in scope (FROM alias + JOIN aliases).
    ///
    /// Returns:
    ///     list[str]: All aliases, empty if none
    #[getter]
    fn table_aliases(&self) -> Vec<String> {
        self.inner.select.from_alias.clone()
    }

    /// Get the list of selected columns — one entry per SELECT-list item.
    ///
    /// The returned list covers every item in the SELECT list, in grammar
    /// order: regular columns, aggregate calls, `similarity()` expressions,
    /// qualified wildcards (`alias.*`), and window functions.
    ///
    /// Returns:
    ///     list[str]: Column identifiers, one per SELECT-list item. Use
    ///     the literal `'*'` for `SELECT *`. Qualified wildcards appear as
    ///     `'alias.*'`. Aliased expressions use the alias (or the bare
    ///     default for un-aliased similarity / window calls).
    ///
    /// Example:
    ///     >>> parsed = VelesQL.parse("SELECT id, name FROM users")
    ///     >>> parsed.columns
    ///     ['id', 'name']
    ///
    ///     >>> parsed = VelesQL.parse(
    ///     ...     "SELECT title, similarity() AS score, "
    ///     ...     "ROW_NUMBER() OVER (ORDER BY score DESC) AS rn FROM docs"
    ///     ... )
    ///     >>> parsed.columns
    ///     ['title', 'score', 'rn']
    ///
    /// Note (v1.13.0 contract completion):
    ///     Versions prior to v1.13.0 silently omitted `similarity()`
    ///     expressions and qualified wildcards from this list for mixed
    ///     SELECT statements. The full list is now returned. Callers that
    ///     relied on the shorter length must update their expectations.
    #[getter]
    fn columns(&self) -> Vec<String> {
        self.inner.select.columns.to_display_names()
    }

    /// Check if DISTINCT modifier is present.
    ///
    /// Returns:
    ///     bool: True if SELECT DISTINCT
    fn has_distinct(&self) -> bool {
        !matches!(
            self.inner.select.distinct,
            velesdb_core::velesql::DistinctMode::None
        )
    }

    /// Check if the query has a WHERE clause.
    ///
    /// Returns:
    ///     bool: True if WHERE clause is present
    fn has_where_clause(&self) -> bool {
        self.inner.select.where_clause.is_some()
    }

    /// Check if the query has an ORDER BY clause.
    ///
    /// Returns:
    ///     bool: True if ORDER BY clause is present
    fn has_order_by(&self) -> bool {
        self.inner.select.order_by.is_some()
    }

    /// Check if the query has a GROUP BY clause.
    ///
    /// Returns:
    ///     bool: True if GROUP BY clause is present
    fn has_group_by(&self) -> bool {
        self.inner.select.group_by.is_some()
    }

    /// Check if the query has a HAVING clause.
    ///
    /// Returns:
    ///     bool: True if HAVING clause is present
    fn has_having(&self) -> bool {
        self.inner.select.having.is_some()
    }

    /// Check if the query has JOINs.
    ///
    /// Returns:
    ///     bool: True if query contains JOIN clauses
    fn has_joins(&self) -> bool {
        !self.inner.select.joins.is_empty()
    }

    /// Check if the query uses FUSION (hybrid search).
    ///
    /// Returns:
    ///     bool: True if USING FUSION is present
    fn has_fusion(&self) -> bool {
        self.inner.select.fusion_clause.is_some()
    }

    /// Check if the query contains vector search (NEAR clause).
    ///
    /// Returns:
    ///     bool: True if query contains vector search
    fn has_vector_search(&self) -> bool {
        if let Some(ref cond) = self.inner.select.where_clause {
            Self::condition_has_vector_search(cond)
        } else {
            false
        }
    }

    /// Get the LIMIT value if present.
    ///
    /// Returns:
    ///     int or None: LIMIT value, or None if not specified
    #[getter]
    fn limit(&self) -> Option<u64> {
        self.inner.select.limit
    }

    /// Get the OFFSET value if present.
    ///
    /// Returns:
    ///     int or None: OFFSET value, or None if not specified
    #[getter]
    fn offset(&self) -> Option<u64> {
        self.inner.select.offset
    }

    /// Get the ORDER BY columns and directions.
    ///
    /// Returns:
    ///     list[tuple[str, str]]: List of (column, direction) tuples
    ///
    /// Example:
    ///     >>> parsed = VelesQL.parse("SELECT * FROM docs ORDER BY date DESC, name ASC")
    ///     >>> print(parsed.order_by)
    ///     [('date', 'DESC'), ('name', 'ASC')]
    #[getter]
    fn order_by(&self) -> Vec<(String, String)> {
        self.inner
            .select
            .order_by
            .as_deref()
            .map_or_else(Vec::new, |items| {
                items.iter().map(|item| item.to_display_pair()).collect()
            })
    }

    /// Get the GROUP BY columns.
    ///
    /// Returns:
    ///     list[str]: List of GROUP BY column names
    #[getter]
    fn group_by(&self) -> Vec<String> {
        match &self.inner.select.group_by {
            Some(gb) => gb.columns.clone(),
            None => Vec::new(),
        }
    }

    /// Get the number of JOIN clauses.
    ///
    /// Returns:
    ///     int: Number of JOINs in the query
    #[getter]
    fn join_count(&self) -> usize {
        self.inner.select.joins.len()
    }

    /// Get a string representation of the parsed query.
    fn __repr__(&self) -> String {
        let query_type = self.query_type_label();
        let coll = self
            .collection_name()
            .unwrap_or_else(|| "<graph>".to_string());
        format!("ParsedStatement({query_type} FROM {coll})")
    }

    /// Get a detailed string representation.
    fn __str__(&self) -> String {
        let mut parts = Vec::new();

        parts.push(format!("Type: {}", self.query_type_label()));

        if let Some(coll) = self.collection_name() {
            parts.push(format!("Collection: {}", coll));
        }

        let cols = self.columns();
        if cols.len() == 1 && cols[0] == "*" {
            parts.push("Columns: *".to_string());
        } else {
            parts.push(format!("Columns: {}", cols.join(", ")));
        }

        if self.has_where_clause() {
            parts.push("WHERE: present".to_string());
        }

        if self.has_vector_search() {
            parts.push("Vector search: yes".to_string());
        }

        if let Some(limit) = self.limit() {
            parts.push(format!("LIMIT: {}", limit));
        }

        parts.join("\n")
    }
}

// Private helpers and module registration are in velesql_helpers.rs

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_select() {
        let result = CoreParser::parse("SELECT * FROM documents LIMIT 10");
        assert!(result.is_ok());
        let query = result.unwrap();
        assert_eq!(query.select.from, "documents");
        assert_eq!(query.select.limit, Some(10));
    }

    #[test]
    fn test_parse_with_where() {
        let result = CoreParser::parse("SELECT * FROM docs WHERE category = 'tech'");
        assert!(result.is_ok());
        let query = result.unwrap();
        assert!(query.select.where_clause.is_some());
    }

    #[test]
    fn test_parse_vector_search() {
        let result = CoreParser::parse("SELECT * FROM docs WHERE vector NEAR $v LIMIT 5");
        assert!(result.is_ok());
        let query = result.unwrap();
        assert!(query.select.where_clause.is_some());
        assert_eq!(query.select.limit, Some(5));
    }

    #[test]
    fn test_parse_invalid_query() {
        let result = CoreParser::parse("SELEC * FROM docs");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_with_order_by() {
        let result = CoreParser::parse("SELECT * FROM docs ORDER BY created_at DESC");
        assert!(result.is_ok());
        let query = result.unwrap();
        assert!(query.select.order_by.is_some());
    }

    #[test]
    fn test_parse_with_distinct() {
        let result = CoreParser::parse("SELECT DISTINCT category FROM products");
        assert!(result.is_ok());
        let query = result.unwrap();
        assert!(!matches!(
            query.select.distinct,
            velesdb_core::velesql::DistinctMode::None
        ));
    }

    #[test]
    fn test_parse_with_join() {
        let result = CoreParser::parse(
            "SELECT * FROM orders JOIN products ON orders.product_id = products.id",
        );
        assert!(result.is_ok());
        let query = result.unwrap();
        assert!(!query.select.joins.is_empty());
    }

    #[test]
    fn test_parse_with_group_by() {
        let result = CoreParser::parse("SELECT category, COUNT(*) FROM products GROUP BY category");
        assert!(result.is_ok());
        let query = result.unwrap();
        assert!(query.select.group_by.is_some());
    }

    #[test]
    fn test_is_ddl_create_collection() {
        let query =
            CoreParser::parse("CREATE COLLECTION docs (dimension = 128, metric = 'cosine')")
                .unwrap();
        let stmt = ParsedStatement { inner: query };
        assert!(stmt.is_ddl());
        assert!(!stmt.is_select());
        assert!(!stmt.is_dml());
    }

    #[test]
    fn test_is_ddl_drop_collection() {
        let query = CoreParser::parse("DROP COLLECTION docs").unwrap();
        let stmt = ParsedStatement { inner: query };
        assert!(stmt.is_ddl());
        assert!(!stmt.is_select());
    }

    #[test]
    fn test_is_delete() {
        let query = CoreParser::parse("DELETE FROM docs WHERE category = 'old'").unwrap();
        let stmt = ParsedStatement { inner: query };
        assert!(stmt.is_delete());
        assert!(stmt.is_dml());
        assert!(!stmt.is_ddl());
        assert!(!stmt.is_select());
    }

    #[test]
    fn test_is_insert_edge() {
        let query =
            CoreParser::parse("INSERT EDGE INTO kg (source = 1, target = 2, label = 'related')")
                .unwrap();
        let stmt = ParsedStatement { inner: query };
        assert!(stmt.is_insert_edge());
        assert!(stmt.is_dml());
        assert!(!stmt.is_ddl());
    }

    #[test]
    fn test_select_is_not_ddl_nor_dml() {
        let query = CoreParser::parse("SELECT * FROM docs LIMIT 10").unwrap();
        let stmt = ParsedStatement { inner: query };
        assert!(!stmt.is_ddl());
        assert!(!stmt.is_dml());
        assert!(!stmt.is_delete());
        assert!(!stmt.is_insert_edge());
        assert!(stmt.is_select());
    }

    #[test]
    fn test_query_type_label_ddl() {
        let query =
            CoreParser::parse("CREATE COLLECTION docs (dimension = 128, metric = 'cosine')")
                .unwrap();
        let stmt = ParsedStatement { inner: query };
        assert_eq!(stmt.query_type_label(), "DDL");
    }

    #[test]
    fn test_query_type_label_dml() {
        let query = CoreParser::parse("DELETE FROM docs WHERE id = 1").unwrap();
        let stmt = ParsedStatement { inner: query };
        assert_eq!(stmt.query_type_label(), "DML");
    }

    #[test]
    fn test_query_type_label_select() {
        let query = CoreParser::parse("SELECT * FROM docs LIMIT 10").unwrap();
        let stmt = ParsedStatement { inner: query };
        assert_eq!(stmt.query_type_label(), "SELECT");
    }
}
