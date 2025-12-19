//! `VelesQL` parser implementation using pest.

use pest::Parser as PestParser;
use pest_derive::Parser;

use super::ast::{
    BetweenCondition, Column, CompareOp, Comparison, Condition, DistanceMetricType, InCondition,
    IsNullCondition, LikeCondition, MatchCondition, Query, SelectColumns, SelectStatement, Value,
    VectorExpr, VectorSearch,
};
use super::error::{ParseError, ParseErrorKind};

#[derive(Parser)]
#[grammar = "velesql/grammar.pest"]
struct VelesQLParser;

/// `VelesQL` query parser.
pub struct Parser;

impl Parser {
    /// Parses a `VelesQL` query string into an AST.
    ///
    /// # Errors
    ///
    /// Returns a `ParseError` if the query is invalid.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use velesdb_core::velesql::Parser;
    ///
    /// let query = Parser::parse("SELECT * FROM documents LIMIT 10")?;
    /// ```
    pub fn parse(input: &str) -> Result<Query, ParseError> {
        let pairs = VelesQLParser::parse(Rule::query, input).map_err(|e| {
            let position = match e.location {
                pest::error::InputLocation::Pos(p) => p,
                pest::error::InputLocation::Span((s, _)) => s,
            };
            ParseError::new(
                ParseErrorKind::SyntaxError,
                position,
                input.chars().take(50).collect::<String>(),
                e.to_string(),
            )
        })?;

        let query_pair = pairs
            .into_iter()
            .next()
            .ok_or_else(|| ParseError::syntax(0, input, "Empty query"))?;

        Self::parse_query(query_pair)
    }

    fn parse_query(pair: pest::iterators::Pair<Rule>) -> Result<Query, ParseError> {
        let mut inner = pair.into_inner();

        let select_pair = inner
            .find(|p| p.as_rule() == Rule::select_stmt)
            .ok_or_else(|| ParseError::syntax(0, "", "Expected SELECT statement"))?;

        let select = Self::parse_select_stmt(select_pair)?;

        Ok(Query { select })
    }

    fn parse_select_stmt(pair: pest::iterators::Pair<Rule>) -> Result<SelectStatement, ParseError> {
        let mut columns = SelectColumns::All;
        let mut from = String::new();
        let mut where_clause = None;
        let mut limit = None;
        let mut offset = None;

        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::select_list => {
                    columns = Self::parse_select_list(inner_pair)?;
                }
                Rule::identifier => {
                    from = inner_pair.as_str().to_string();
                }
                Rule::where_clause => {
                    where_clause = Some(Self::parse_where_clause(inner_pair)?);
                }
                Rule::limit_clause => {
                    limit = Some(Self::parse_limit_clause(inner_pair)?);
                }
                Rule::offset_clause => {
                    offset = Some(Self::parse_offset_clause(inner_pair)?);
                }
                _ => {}
            }
        }

        Ok(SelectStatement {
            columns,
            from,
            where_clause,
            limit,
            offset,
        })
    }

    fn parse_select_list(pair: pest::iterators::Pair<Rule>) -> Result<SelectColumns, ParseError> {
        let inner = pair.into_inner().next();

        match inner {
            Some(p) if p.as_rule() == Rule::column_list => {
                let columns = Self::parse_column_list(p)?;
                Ok(SelectColumns::Columns(columns))
            }
            _ => Ok(SelectColumns::All),
        }
    }

    fn parse_column_list(pair: pest::iterators::Pair<Rule>) -> Result<Vec<Column>, ParseError> {
        let mut columns = Vec::new();

        for col_pair in pair.into_inner() {
            if col_pair.as_rule() == Rule::column {
                columns.push(Self::parse_column(col_pair)?);
            }
        }

        Ok(columns)
    }

    fn parse_column(pair: pest::iterators::Pair<Rule>) -> Result<Column, ParseError> {
        let mut inner = pair.into_inner();
        let name_pair = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected column name"))?;

        let name = Self::parse_column_name(&name_pair);
        let alias = inner.next().map(|p| p.as_str().to_string());

        Ok(Column { name, alias })
    }

    fn parse_column_name(pair: &pest::iterators::Pair<Rule>) -> String {
        // column_name is atomic (@), so we get the full string directly
        pair.as_str().to_string()
    }

    fn parse_where_clause(pair: pest::iterators::Pair<Rule>) -> Result<Condition, ParseError> {
        let or_expr = pair
            .into_inner()
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected condition"))?;

        Self::parse_or_expr(or_expr)
    }

    fn parse_or_expr(pair: pest::iterators::Pair<Rule>) -> Result<Condition, ParseError> {
        let mut inner = pair.into_inner().peekable();

        let first = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected condition"))?;

        let mut result = Self::parse_and_expr(first)?;

        for and_expr in inner {
            let right = Self::parse_and_expr(and_expr)?;
            result = Condition::Or(Box::new(result), Box::new(right));
        }

        Ok(result)
    }

    fn parse_and_expr(pair: pest::iterators::Pair<Rule>) -> Result<Condition, ParseError> {
        let mut inner = pair.into_inner().peekable();

        let first = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected condition"))?;

        let mut result = Self::parse_primary_expr(first)?;

        for primary in inner {
            let right = Self::parse_primary_expr(primary)?;
            result = Condition::And(Box::new(result), Box::new(right));
        }

        Ok(result)
    }

    fn parse_primary_expr(pair: pest::iterators::Pair<Rule>) -> Result<Condition, ParseError> {
        let inner = pair
            .into_inner()
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected primary condition"))?;

        match inner.as_rule() {
            Rule::or_expr => {
                let cond = Self::parse_or_expr(inner)?;
                Ok(Condition::Group(Box::new(cond)))
            }
            Rule::vector_search => Self::parse_vector_search(inner),
            Rule::match_expr => Self::parse_match_expr(inner),
            Rule::in_expr => Self::parse_in_expr(inner),
            Rule::between_expr => Self::parse_between_expr(inner),
            Rule::like_expr => Self::parse_like_expr(inner),
            Rule::is_null_expr => Self::parse_is_null_expr(inner),
            Rule::compare_expr => Self::parse_compare_expr(inner),
            _ => Err(ParseError::syntax(
                0,
                inner.as_str(),
                "Unknown condition type",
            )),
        }
    }

    fn parse_vector_search(pair: pest::iterators::Pair<Rule>) -> Result<Condition, ParseError> {
        let mut metric = DistanceMetricType::default();
        let mut vector = None;

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::metric => {
                    metric = match inner.as_str().to_uppercase().as_str() {
                        "EUCLIDEAN" => DistanceMetricType::Euclidean,
                        "DOT" => DistanceMetricType::Dot,
                        // "COSINE" and any other value defaults to Cosine
                        _ => DistanceMetricType::Cosine,
                    };
                }
                Rule::vector_value => {
                    vector = Some(Self::parse_vector_value(inner)?);
                }
                _ => {}
            }
        }

        let vector =
            vector.ok_or_else(|| ParseError::syntax(0, "", "Expected vector expression"))?;

        Ok(Condition::VectorSearch(VectorSearch { metric, vector }))
    }

    fn parse_vector_value(pair: pest::iterators::Pair<Rule>) -> Result<VectorExpr, ParseError> {
        let inner = pair
            .into_inner()
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected vector expression"))?;

        match inner.as_rule() {
            Rule::vector_literal => {
                let values: Result<Vec<f32>, _> = inner
                    .into_inner()
                    .filter(|p| p.as_rule() == Rule::float)
                    .map(|p| {
                        p.as_str()
                            .parse::<f32>()
                            .map_err(|_| ParseError::syntax(0, p.as_str(), "Invalid float value"))
                    })
                    .collect();
                Ok(VectorExpr::Literal(values?))
            }
            Rule::parameter => {
                let name = inner.as_str().trim_start_matches('$').to_string();
                Ok(VectorExpr::Parameter(name))
            }
            _ => Err(ParseError::syntax(
                0,
                inner.as_str(),
                "Expected vector literal or parameter",
            )),
        }
    }

    fn parse_match_expr(pair: pest::iterators::Pair<Rule>) -> Result<Condition, ParseError> {
        let mut inner = pair.into_inner();

        let column = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected column name"))?
            .as_str()
            .to_string();

        let query = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected match query"))?
            .as_str()
            .trim_matches('\'')
            .to_string();

        Ok(Condition::Match(MatchCondition { column, query }))
    }

    fn parse_in_expr(pair: pest::iterators::Pair<Rule>) -> Result<Condition, ParseError> {
        let mut inner = pair.into_inner();

        let column = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected column name"))?
            .as_str()
            .to_string();

        let value_list = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected value list"))?;

        let values: Result<Vec<Value>, _> = value_list
            .into_inner()
            .filter(|p| p.as_rule() == Rule::value)
            .map(Self::parse_value)
            .collect();

        Ok(Condition::In(InCondition {
            column,
            values: values?,
        }))
    }

    fn parse_between_expr(pair: pest::iterators::Pair<Rule>) -> Result<Condition, ParseError> {
        let mut inner = pair.into_inner();

        let column = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected column name"))?
            .as_str()
            .to_string();

        let low = Self::parse_value(
            inner
                .next()
                .ok_or_else(|| ParseError::syntax(0, "", "Expected low value"))?,
        )?;

        let high = Self::parse_value(
            inner
                .next()
                .ok_or_else(|| ParseError::syntax(0, "", "Expected high value"))?,
        )?;

        Ok(Condition::Between(BetweenCondition { column, low, high }))
    }

    fn parse_like_expr(pair: pest::iterators::Pair<Rule>) -> Result<Condition, ParseError> {
        let mut inner = pair.into_inner();

        let column = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected column name"))?
            .as_str()
            .to_string();

        let pattern = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected pattern"))?
            .as_str()
            .trim_matches('\'')
            .to_string();

        Ok(Condition::Like(LikeCondition { column, pattern }))
    }

    fn parse_is_null_expr(pair: pest::iterators::Pair<Rule>) -> Result<Condition, ParseError> {
        let mut column = String::new();
        let mut has_not = false;

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::identifier => {
                    column = inner.as_str().to_string();
                }
                Rule::not_kw => {
                    has_not = true;
                }
                _ => {}
            }
        }

        if column.is_empty() {
            return Err(ParseError::syntax(0, "", "Expected column name in IS NULL"));
        }

        Ok(Condition::IsNull(IsNullCondition {
            column,
            is_null: !has_not,
        }))
    }

    fn parse_compare_expr(pair: pest::iterators::Pair<Rule>) -> Result<Condition, ParseError> {
        let mut inner = pair.into_inner();

        let column = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected column name"))?
            .as_str()
            .to_string();

        let op_pair = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected operator"))?;

        let operator = match op_pair.as_str() {
            "=" => CompareOp::Eq,
            "!=" | "<>" => CompareOp::NotEq,
            ">" => CompareOp::Gt,
            ">=" => CompareOp::Gte,
            "<" => CompareOp::Lt,
            "<=" => CompareOp::Lte,
            _ => return Err(ParseError::syntax(0, op_pair.as_str(), "Invalid operator")),
        };

        let value = Self::parse_value(
            inner
                .next()
                .ok_or_else(|| ParseError::syntax(0, "", "Expected value"))?,
        )?;

        Ok(Condition::Comparison(Comparison {
            column,
            operator,
            value,
        }))
    }

    fn parse_value(pair: pest::iterators::Pair<Rule>) -> Result<Value, ParseError> {
        let inner = pair
            .into_inner()
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected value"))?;

        match inner.as_rule() {
            Rule::integer => {
                let v = inner
                    .as_str()
                    .parse::<i64>()
                    .map_err(|_| ParseError::syntax(0, inner.as_str(), "Invalid integer"))?;
                Ok(Value::Integer(v))
            }
            Rule::float => {
                let v = inner
                    .as_str()
                    .parse::<f64>()
                    .map_err(|_| ParseError::syntax(0, inner.as_str(), "Invalid float"))?;
                Ok(Value::Float(v))
            }
            Rule::string => {
                let s = inner.as_str().trim_matches('\'').to_string();
                Ok(Value::String(s))
            }
            Rule::boolean => {
                let b = inner.as_str().to_uppercase() == "TRUE";
                Ok(Value::Boolean(b))
            }
            Rule::null_value => Ok(Value::Null),
            Rule::parameter => {
                let name = inner.as_str().trim_start_matches('$').to_string();
                Ok(Value::Parameter(name))
            }
            _ => Err(ParseError::syntax(0, inner.as_str(), "Unknown value type")),
        }
    }

    fn parse_limit_clause(pair: pest::iterators::Pair<Rule>) -> Result<u64, ParseError> {
        let int_pair = pair
            .into_inner()
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected integer for LIMIT"))?;

        int_pair
            .as_str()
            .parse::<u64>()
            .map_err(|_| ParseError::syntax(0, int_pair.as_str(), "Invalid LIMIT value"))
    }

    fn parse_offset_clause(pair: pest::iterators::Pair<Rule>) -> Result<u64, ParseError> {
        let int_pair = pair
            .into_inner()
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected integer for OFFSET"))?;

        int_pair
            .as_str()
            .parse::<u64>()
            .map_err(|_| ParseError::syntax(0, int_pair.as_str(), "Invalid OFFSET value"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========== Basic SELECT tests ==========

    #[test]
    fn test_parse_select_all() {
        let query = Parser::parse("SELECT * FROM documents").unwrap();
        assert_eq!(query.select.columns, SelectColumns::All);
        assert_eq!(query.select.from, "documents");
        assert!(query.select.where_clause.is_none());
        assert!(query.select.limit.is_none());
    }

    #[test]
    fn test_parse_select_with_limit() {
        let query = Parser::parse("SELECT * FROM documents LIMIT 10").unwrap();
        assert_eq!(query.select.limit, Some(10));
    }

    #[test]
    fn test_parse_select_with_offset() {
        let query = Parser::parse("SELECT * FROM documents LIMIT 10 OFFSET 5").unwrap();
        assert_eq!(query.select.limit, Some(10));
        assert_eq!(query.select.offset, Some(5));
    }

    #[test]
    fn test_parse_select_columns() {
        let query = Parser::parse("SELECT id, score FROM documents").unwrap();
        match query.select.columns {
            SelectColumns::Columns(cols) => {
                assert_eq!(cols.len(), 2);
                assert_eq!(cols[0].name, "id");
                assert_eq!(cols[1].name, "score");
            }
            SelectColumns::All => panic!("Expected columns list"),
        }
    }

    #[test]
    fn test_parse_select_nested_column() {
        let query = Parser::parse("SELECT payload.title FROM documents").unwrap();
        match query.select.columns {
            SelectColumns::Columns(cols) => {
                assert_eq!(cols[0].name, "payload.title");
            }
            SelectColumns::All => panic!("Expected columns list"),
        }
    }

    // ========== Vector search tests ==========

    #[test]
    fn test_parse_vector_near_parameter() {
        let query = Parser::parse("SELECT * FROM documents WHERE vector NEAR $v").unwrap();
        match query.select.where_clause {
            Some(Condition::VectorSearch(vs)) => {
                assert_eq!(vs.metric, DistanceMetricType::Cosine);
                assert_eq!(vs.vector, VectorExpr::Parameter("v".to_string()));
            }
            _ => panic!("Expected vector search condition"),
        }
    }

    #[test]
    fn test_parse_vector_near_literal() {
        let query = Parser::parse("SELECT * FROM docs WHERE vector NEAR [0.1, 0.2, 0.3]").unwrap();
        match query.select.where_clause {
            Some(Condition::VectorSearch(vs)) => match vs.vector {
                VectorExpr::Literal(v) => {
                    assert_eq!(v.len(), 3);
                    assert!((v[0] - 0.1).abs() < 0.001);
                }
                VectorExpr::Parameter(_) => panic!("Expected literal vector"),
            },
            _ => panic!("Expected vector search condition"),
        }
    }

    #[test]
    fn test_parse_vector_near_euclidean() {
        let query = Parser::parse("SELECT * FROM docs WHERE vector NEAR EUCLIDEAN $v").unwrap();
        match query.select.where_clause {
            Some(Condition::VectorSearch(vs)) => {
                assert_eq!(vs.metric, DistanceMetricType::Euclidean);
            }
            _ => panic!("Expected vector search condition"),
        }
    }

    #[test]
    fn test_parse_vector_near_dot() {
        let query = Parser::parse("SELECT * FROM docs WHERE vector NEAR DOT $v").unwrap();
        match query.select.where_clause {
            Some(Condition::VectorSearch(vs)) => {
                assert_eq!(vs.metric, DistanceMetricType::Dot);
            }
            _ => panic!("Expected vector search condition"),
        }
    }

    // ========== Comparison tests ==========

    #[test]
    fn test_parse_comparison_eq_string() {
        let query = Parser::parse("SELECT * FROM docs WHERE category = 'tech'").unwrap();
        match query.select.where_clause {
            Some(Condition::Comparison(c)) => {
                assert_eq!(c.column, "category");
                assert_eq!(c.operator, CompareOp::Eq);
                assert_eq!(c.value, Value::String("tech".to_string()));
            }
            _ => panic!("Expected comparison condition"),
        }
    }

    #[test]
    fn test_parse_comparison_gt_integer() {
        let query = Parser::parse("SELECT * FROM docs WHERE price > 100").unwrap();
        match query.select.where_clause {
            Some(Condition::Comparison(c)) => {
                assert_eq!(c.column, "price");
                assert_eq!(c.operator, CompareOp::Gt);
                assert_eq!(c.value, Value::Integer(100));
            }
            _ => panic!("Expected comparison condition"),
        }
    }

    #[test]
    fn test_parse_comparison_neq() {
        let query = Parser::parse("SELECT * FROM docs WHERE status != 'deleted'").unwrap();
        match query.select.where_clause {
            Some(Condition::Comparison(c)) => {
                assert_eq!(c.operator, CompareOp::NotEq);
            }
            _ => panic!("Expected comparison condition"),
        }
    }

    // ========== Logical operators tests ==========

    #[test]
    fn test_parse_and_condition() {
        let query =
            Parser::parse("SELECT * FROM docs WHERE category = 'tech' AND price > 100").unwrap();
        match query.select.where_clause {
            Some(Condition::And(_, _)) => {}
            _ => panic!("Expected AND condition"),
        }
    }

    #[test]
    fn test_parse_or_condition() {
        let query =
            Parser::parse("SELECT * FROM docs WHERE category = 'tech' OR category = 'science'")
                .unwrap();
        match query.select.where_clause {
            Some(Condition::Or(_, _)) => {}
            _ => panic!("Expected OR condition"),
        }
    }

    #[test]
    fn test_parse_vector_with_filter() {
        let query =
            Parser::parse("SELECT * FROM docs WHERE vector NEAR $v AND category = 'tech' LIMIT 10")
                .unwrap();
        match query.select.where_clause {
            Some(Condition::And(left, _)) => match *left {
                Condition::VectorSearch(_) => {}
                _ => panic!("Expected vector search on left"),
            },
            _ => panic!("Expected AND condition"),
        }
        assert_eq!(query.select.limit, Some(10));
    }

    // ========== IN/BETWEEN/LIKE tests ==========

    #[test]
    fn test_parse_in_condition() {
        let query =
            Parser::parse("SELECT * FROM docs WHERE category IN ('tech', 'science')").unwrap();
        match query.select.where_clause {
            Some(Condition::In(c)) => {
                assert_eq!(c.column, "category");
                assert_eq!(c.values.len(), 2);
            }
            _ => panic!("Expected IN condition"),
        }
    }

    #[test]
    fn test_parse_between_condition() {
        let query = Parser::parse("SELECT * FROM docs WHERE price BETWEEN 10 AND 100").unwrap();
        match query.select.where_clause {
            Some(Condition::Between(c)) => {
                assert_eq!(c.column, "price");
                assert_eq!(c.low, Value::Integer(10));
                assert_eq!(c.high, Value::Integer(100));
            }
            _ => panic!("Expected BETWEEN condition"),
        }
    }

    #[test]
    fn test_parse_like_condition() {
        let query = Parser::parse("SELECT * FROM docs WHERE title LIKE '%rust%'").unwrap();
        match query.select.where_clause {
            Some(Condition::Like(c)) => {
                assert_eq!(c.column, "title");
                assert_eq!(c.pattern, "%rust%");
            }
            _ => panic!("Expected LIKE condition"),
        }
    }

    // ========== IS NULL tests ==========

    #[test]
    fn test_parse_is_null() {
        let query = Parser::parse("SELECT * FROM docs WHERE deleted_at IS NULL").unwrap();
        match query.select.where_clause {
            Some(Condition::IsNull(c)) => {
                assert_eq!(c.column, "deleted_at");
                assert!(c.is_null);
            }
            _ => panic!("Expected IS NULL condition"),
        }
    }

    #[test]
    fn test_parse_is_not_null() {
        let query = Parser::parse("SELECT * FROM docs WHERE title IS NOT NULL").unwrap();
        match query.select.where_clause {
            Some(Condition::IsNull(c)) => {
                assert_eq!(c.column, "title");
                assert!(!c.is_null);
            }
            _ => panic!("Expected IS NOT NULL condition"),
        }
    }

    // ========== Error tests ==========

    #[test]
    fn test_parse_syntax_error() {
        let result = Parser::parse("SELEC * FROM docs");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_missing_from() {
        let result = Parser::parse("SELECT * docs");
        assert!(result.is_err());
    }

    // ========== Case insensitivity tests ==========

    #[test]
    fn test_parse_case_insensitive() {
        let query = Parser::parse("select * from documents where vector near $v limit 10").unwrap();
        assert_eq!(query.select.from, "documents");
        assert_eq!(query.select.limit, Some(10));
    }
}
