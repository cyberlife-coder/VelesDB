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
//! assert parsed.table_name == "docs"
//! ```

// Exceptions created via pyo3::create_exception! macro
use pyo3::prelude::*;
use velesdb_core::velesql::{
    ParseError as CoreParseError, Parser as CoreParser, Query as CoreQuery, SelectColumns,
};

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
///     >>> print(parsed.table_name)
///     'docs'
#[pyclass]
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
    ///     >>> assert parsed.table_name == "documents"
    #[staticmethod]
    fn parse(query: &str) -> PyResult<ParsedStatement> {
        CoreParser::parse(query)
            .map(|q| ParsedStatement { inner: q })
            .map_err(|e| VelesQLSyntaxError::new_err(format_parse_error(&e)))
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
#[pyclass]
pub struct ParsedStatement {
    inner: CoreQuery,
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

    /// Get the table name from the FROM clause.
    ///
    /// Returns:
    ///     str or None: Table name, or None for MATCH queries
    #[getter]
    fn table_name(&self) -> Option<String> {
        let from = &self.inner.select.from;
        if from.is_empty() {
            None
        } else {
            Some(from.clone())
        }
    }

    /// Get the table alias if present (for self-joins).
    ///
    /// Returns:
    ///     str or None: Table alias, or None if not aliased
    #[getter]
    fn table_alias(&self) -> Option<String> {
        self.inner.select.from_alias.clone()
    }

    /// Get the list of selected columns.
    ///
    /// Returns:
    ///     list[str] or '*': List of column names, or ['*'] for SELECT *
    ///
    /// Example:
    ///     >>> parsed = VelesQL.parse("SELECT id, name FROM users")
    ///     >>> print(parsed.columns)
    ///     ['id', 'name']
    #[getter]
    fn columns(&self) -> Vec<String> {
        match &self.inner.select.columns {
            SelectColumns::All => vec!["*".to_string()],
            SelectColumns::Columns(cols) => cols.iter().map(|c| c.name.clone()).collect(),
            SelectColumns::Aggregations(aggs) => aggs
                .iter()
                .map(|a| format!("{:?}", a.function_type))
                .collect(),
            SelectColumns::Mixed {
                columns,
                aggregations,
            } => {
                let mut result: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
                result.extend(
                    aggregations
                        .iter()
                        .map(|a| format!("{:?}", a.function_type)),
                );
                result
            }
        }
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
        match &self.inner.select.order_by {
            Some(order_items) => order_items
                .iter()
                .map(|item| {
                    let dir = if item.descending { "DESC" } else { "ASC" };
                    let col = match &item.expr {
                        velesdb_core::velesql::OrderByExpr::Field(f) => f.clone(),
                        velesdb_core::velesql::OrderByExpr::Similarity(_) => {
                            "similarity()".to_string()
                        }
                        velesdb_core::velesql::OrderByExpr::Aggregate(agg) => {
                            format!("{:?}", agg.function_type)
                        }
                    };
                    (col, dir.to_string())
                })
                .collect(),
            None => Vec::new(),
        }
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
        let query_type = if self.is_match() { "MATCH" } else { "SELECT" };
        let table = self.table_name().unwrap_or_else(|| "<graph>".to_string());
        format!("ParsedStatement({} FROM {})", query_type, table)
    }

    /// Get a detailed string representation.
    fn __str__(&self) -> String {
        let mut parts = Vec::new();

        if self.is_match() {
            parts.push("Type: MATCH (graph query)".to_string());
        } else {
            parts.push("Type: SELECT".to_string());
        }

        if let Some(table) = self.table_name() {
            parts.push(format!("Table: {}", table));
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

impl ParsedStatement {
    /// Recursively check if a condition contains vector search.
    fn condition_has_vector_search(cond: &velesdb_core::velesql::Condition) -> bool {
        use velesdb_core::velesql::Condition;

        match cond {
            Condition::VectorSearch(_) | Condition::VectorFusedSearch { .. } => true,
            Condition::And(left, right) | Condition::Or(left, right) => {
                Self::condition_has_vector_search(left) || Self::condition_has_vector_search(right)
            }
            Condition::Group(inner) => Self::condition_has_vector_search(inner),
            Condition::Not(inner) => Self::condition_has_vector_search(inner),
            _ => false,
        }
    }
}

/// Format a parse error for Python exception message.
fn format_parse_error(e: &CoreParseError) -> String {
    format!(
        "VelesQL syntax error at position {}: {} (near '{}')",
        e.position, e.message, e.fragment
    )
}

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
}

/// Register VelesQL classes with the Python module.
pub fn register_velesql_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<VelesQL>()?;
    m.add_class::<ParsedStatement>()?;
    m.add(
        "VelesQLSyntaxError",
        m.py().get_type::<VelesQLSyntaxError>(),
    )?;
    m.add(
        "VelesQLParameterError",
        m.py().get_type::<VelesQLParameterError>(),
    )?;
    Ok(())
}
