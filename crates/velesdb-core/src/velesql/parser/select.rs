//! SELECT statement parsing.

use super::Rule;
use crate::velesql::ast::{
    AggregateArg, AggregateFunction, AggregateType, Column, ColumnRef, CompareOp, CompoundQuery,
    GroupByClause, HavingClause, HavingCondition, JoinClause, JoinCondition, OrderByExpr, Query,
    SelectColumns, SelectOrderBy, SelectStatement, SetOperator, SimilarityOrderBy,
};
use crate::velesql::error::ParseError;
use crate::velesql::Parser;

impl Parser {
    pub(crate) fn parse_query(pair: pest::iterators::Pair<Rule>) -> Result<Query, ParseError> {
        let mut inner = pair.into_inner();

        // Find compound_query (EPIC-040 US-006)
        let compound_pair = inner
            .find(|p| p.as_rule() == Rule::compound_query)
            .ok_or_else(|| ParseError::syntax(0, "", "Expected query"))?;

        Self::parse_compound_query(compound_pair)
    }

    /// Parse compound query with optional set operator (EPIC-040 US-006).
    fn parse_compound_query(pair: pest::iterators::Pair<Rule>) -> Result<Query, ParseError> {
        let mut select_stmts = Vec::new();
        let mut set_op = None;

        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::select_stmt => {
                    select_stmts.push(Self::parse_select_stmt(inner_pair)?);
                }
                Rule::set_operator => {
                    set_op = Some(Self::parse_set_operator(inner_pair.as_str()));
                }
                _ => {}
            }
        }

        let select = select_stmts
            .first()
            .cloned()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected SELECT statement"))?;

        let compound = if let (Some(op), Some(right)) = (set_op, select_stmts.get(1).cloned()) {
            Some(CompoundQuery {
                operator: op,
                right: Box::new(right),
            })
        } else {
            None
        };

        Ok(Query { select, compound })
    }

    /// Parse set operator (UNION, UNION ALL, INTERSECT, EXCEPT).
    fn parse_set_operator(text: &str) -> SetOperator {
        let upper = text.to_uppercase();
        if upper.contains("UNION") && upper.contains("ALL") {
            SetOperator::UnionAll
        } else if upper.contains("UNION") {
            SetOperator::Union
        } else if upper.contains("INTERSECT") {
            SetOperator::Intersect
        } else {
            SetOperator::Except
        }
    }

    pub(crate) fn parse_select_stmt(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<SelectStatement, ParseError> {
        let mut columns = SelectColumns::All;
        let mut from = String::new();
        let mut joins = Vec::new();
        let mut where_clause = None;
        let mut order_by = None;
        let mut limit = None;
        let mut offset = None;
        let mut with_clause = None;
        let mut group_by = None;
        let mut having = None;
        let mut fusion_clause = None;

        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::select_list => {
                    columns = Self::parse_select_list(inner_pair)?;
                }
                Rule::identifier => {
                    from = inner_pair.as_str().to_string();
                }
                Rule::join_clause => {
                    joins.push(Self::parse_join_clause(inner_pair)?);
                }
                Rule::where_clause => {
                    where_clause = Some(Self::parse_where_clause(inner_pair)?);
                }
                Rule::group_by_clause => {
                    group_by = Some(Self::parse_group_by_clause(inner_pair));
                }
                Rule::having_clause => {
                    having = Some(Self::parse_having_clause(inner_pair)?);
                }
                Rule::order_by_clause => {
                    order_by = Some(Self::parse_order_by_clause(inner_pair)?);
                }
                Rule::limit_clause => {
                    limit = Some(Self::parse_limit_clause(inner_pair)?);
                }
                Rule::offset_clause => {
                    offset = Some(Self::parse_offset_clause(inner_pair)?);
                }
                Rule::with_clause => {
                    with_clause = Some(Self::parse_with_clause(inner_pair)?);
                }
                Rule::using_fusion_clause => {
                    fusion_clause = Some(Self::parse_using_fusion_clause(inner_pair));
                }
                _ => {}
            }
        }

        Ok(SelectStatement {
            columns,
            from,
            joins,
            where_clause,
            order_by,
            limit,
            offset,
            with_clause,
            group_by,
            having,
            fusion_clause,
        })
    }

    pub(crate) fn parse_order_by_clause(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Vec<SelectOrderBy>, ParseError> {
        let mut items = Vec::new();

        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::order_by_item {
                items.push(Self::parse_order_by_item(inner_pair)?);
            }
        }

        Ok(items)
    }

    pub(crate) fn parse_order_by_item(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<SelectOrderBy, ParseError> {
        let mut expr = None;
        let mut descending = None;
        let mut is_similarity = false;

        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::order_by_expr => {
                    let (parsed_expr, sim) = Self::parse_order_by_expr(inner_pair)?;
                    expr = Some(parsed_expr);
                    is_similarity = sim;
                }
                Rule::sort_direction => {
                    let dir = inner_pair.as_str().to_uppercase();
                    descending = Some(dir == "DESC");
                }
                _ => {}
            }
        }

        let expr = expr.ok_or_else(|| ParseError::syntax(0, "", "Expected ORDER BY expression"))?;

        // Default: DESC for similarity (highest first), ASC for fields
        let descending = descending.unwrap_or(is_similarity);

        Ok(SelectOrderBy { expr, descending })
    }

    pub(crate) fn parse_order_by_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<(OrderByExpr, bool), ParseError> {
        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::order_by_similarity => {
                    let sim = Self::parse_order_by_similarity(inner_pair)?;
                    return Ok((OrderByExpr::Similarity(sim), true));
                }
                Rule::aggregate_function => {
                    // EPIC-040 US-002: Support ORDER BY with aggregate functions
                    let agg = Self::parse_aggregate_function_only(inner_pair)?;
                    return Ok((OrderByExpr::Aggregate(agg), false));
                }
                Rule::identifier => {
                    return Ok((OrderByExpr::Field(inner_pair.as_str().to_string()), false));
                }
                _ => {}
            }
        }

        Err(ParseError::syntax(0, "", "Invalid ORDER BY expression"))
    }

    pub(crate) fn parse_order_by_similarity(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<SimilarityOrderBy, ParseError> {
        let mut field = None;
        let mut vector = None;

        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::similarity_field => {
                    field = Some(inner_pair.as_str().to_string());
                }
                Rule::vector_value => {
                    vector = Some(Self::parse_vector_value(inner_pair)?);
                }
                _ => {}
            }
        }

        let field = field
            .ok_or_else(|| ParseError::syntax(0, "", "Expected field in ORDER BY similarity"))?;
        let vector = vector
            .ok_or_else(|| ParseError::syntax(0, "", "Expected vector in ORDER BY similarity"))?;

        Ok(SimilarityOrderBy { field, vector })
    }

    pub(crate) fn parse_select_list(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<SelectColumns, ParseError> {
        let inner = pair.into_inner().next();

        match inner {
            Some(p) if p.as_rule() == Rule::select_item_list => {
                // Parse mixed list of columns and aggregations
                let (columns, aggs) = Self::parse_select_item_list(p)?;
                if aggs.is_empty() {
                    Ok(SelectColumns::Columns(columns))
                } else if columns.is_empty() {
                    Ok(SelectColumns::Aggregations(aggs))
                } else {
                    // Mixed: columns + aggregations (for GROUP BY)
                    Ok(SelectColumns::Mixed {
                        columns,
                        aggregations: aggs,
                    })
                }
            }
            _ => Ok(SelectColumns::All),
        }
    }

    /// Parse a mixed list of columns and aggregations.
    pub(crate) fn parse_select_item_list(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<(Vec<Column>, Vec<AggregateFunction>), ParseError> {
        let mut columns = Vec::new();
        let mut aggs = Vec::new();

        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::select_item {
                // Each select_item can be aggregation_item or column
                for item in inner_pair.into_inner() {
                    match item.as_rule() {
                        Rule::aggregation_item => {
                            aggs.push(Self::parse_aggregation_item(item)?);
                        }
                        Rule::column => {
                            columns.push(Self::parse_column(item)?);
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok((columns, aggs))
    }

    /// Parse a list of aggregate functions.
    #[allow(dead_code)]
    pub(crate) fn parse_aggregation_list(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Vec<AggregateFunction>, ParseError> {
        let mut aggs = Vec::new();
        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::aggregation_item {
                aggs.push(Self::parse_aggregation_item(inner_pair)?);
            }
        }
        Ok(aggs)
    }

    /// Parse a single aggregation item (e.g., COUNT(*) AS total).
    pub(crate) fn parse_aggregation_item(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<AggregateFunction, ParseError> {
        let mut function_type = None;
        let mut argument = None;
        let mut alias = None;

        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::aggregate_function => {
                    let (ft, arg) = Self::parse_aggregate_function(inner_pair)?;
                    function_type = Some(ft);
                    argument = Some(arg);
                }
                Rule::identifier => {
                    alias = Some(inner_pair.as_str().to_string());
                }
                _ => {}
            }
        }

        let function_type = function_type
            .ok_or_else(|| ParseError::syntax(0, "", "Expected aggregate function"))?;
        let argument =
            argument.ok_or_else(|| ParseError::syntax(0, "", "Expected aggregate argument"))?;

        Ok(AggregateFunction {
            function_type,
            argument,
            alias,
        })
    }

    /// Parse an aggregate function (e.g., COUNT(*), SUM(price)).
    pub(crate) fn parse_aggregate_function(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<(AggregateType, AggregateArg), ParseError> {
        let mut agg_type = None;
        let mut arg = None;

        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::aggregate_type => {
                    agg_type = Some(Self::parse_aggregate_type(&inner_pair)?);
                }
                Rule::aggregate_arg => {
                    arg = Some(Self::parse_aggregate_arg(inner_pair));
                }
                _ => {}
            }
        }

        let agg_type =
            agg_type.ok_or_else(|| ParseError::syntax(0, "", "Expected aggregate type"))?;
        let arg = arg.ok_or_else(|| ParseError::syntax(0, "", "Expected aggregate argument"))?;

        Ok((agg_type, arg))
    }

    /// Parse aggregate type (COUNT, SUM, AVG, MIN, MAX).
    pub(crate) fn parse_aggregate_type(
        pair: &pest::iterators::Pair<Rule>,
    ) -> Result<AggregateType, ParseError> {
        let type_str = pair.as_str().to_uppercase();
        match type_str.as_str() {
            "COUNT" => Ok(AggregateType::Count),
            "SUM" => Ok(AggregateType::Sum),
            "AVG" => Ok(AggregateType::Avg),
            "MIN" => Ok(AggregateType::Min),
            "MAX" => Ok(AggregateType::Max),
            other => Err(ParseError::syntax(0, other, "Unknown aggregate function")),
        }
    }

    /// Parse aggregate argument (* or column name).
    pub(crate) fn parse_aggregate_arg(pair: pest::iterators::Pair<Rule>) -> AggregateArg {
        let inner = pair.into_inner().next();
        match inner {
            Some(p) if p.as_rule() == Rule::column_name => {
                AggregateArg::Column(p.as_str().to_string())
            }
            _ => AggregateArg::Wildcard,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn parse_column_list(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Vec<Column>, ParseError> {
        let mut columns = Vec::new();

        for col_pair in pair.into_inner() {
            if col_pair.as_rule() == Rule::column {
                columns.push(Self::parse_column(col_pair)?);
            }
        }

        Ok(columns)
    }

    pub(crate) fn parse_column(pair: pest::iterators::Pair<Rule>) -> Result<Column, ParseError> {
        let mut inner = pair.into_inner();
        let name_pair = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected column name"))?;

        let name = Self::parse_column_name(&name_pair);
        let alias = inner.next().map(|p| p.as_str().to_string());

        Ok(Column { name, alias })
    }

    pub(crate) fn parse_column_name(pair: &pest::iterators::Pair<Rule>) -> String {
        // column_name is atomic (@), so we get the full string directly
        pair.as_str().to_string()
    }

    /// Parse JOIN clause (EPIC-031 US-004, extended EPIC-040 US-003).
    pub(crate) fn parse_join_clause(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<JoinClause, ParseError> {
        let mut join_type = crate::velesql::JoinType::Inner; // Default
        let mut table = String::new();
        let mut alias = None;
        let mut condition = None;
        let mut using_columns = None;

        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::join_type => {
                    join_type = Self::parse_join_type(inner_pair.as_str());
                }
                Rule::identifier => {
                    table = inner_pair.as_str().to_string();
                }
                Rule::alias_clause => {
                    for alias_inner in inner_pair.into_inner() {
                        if alias_inner.as_rule() == Rule::identifier {
                            alias = Some(alias_inner.as_str().to_string());
                        }
                    }
                }
                Rule::join_spec => {
                    for spec_inner in inner_pair.into_inner() {
                        match spec_inner.as_rule() {
                            Rule::on_clause => {
                                for on_inner in spec_inner.into_inner() {
                                    if on_inner.as_rule() == Rule::join_condition {
                                        condition = Some(Self::parse_join_condition(on_inner)?);
                                    }
                                }
                            }
                            Rule::using_clause => {
                                let cols: Vec<String> = spec_inner
                                    .into_inner()
                                    .filter(|p| p.as_rule() == Rule::identifier)
                                    .map(|p| p.as_str().to_string())
                                    .collect();
                                using_columns = Some(cols);
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        // Either condition or using_columns must be present
        if condition.is_none() && using_columns.is_none() {
            return Err(ParseError::syntax(
                0,
                "",
                "JOIN clause requires ON or USING",
            ));
        }

        Ok(JoinClause {
            join_type,
            table,
            alias,
            condition,
            using_columns,
        })
    }

    /// Parse JOIN type (LEFT, RIGHT, FULL, INNER).
    fn parse_join_type(text: &str) -> crate::velesql::JoinType {
        let text = text.to_uppercase();
        if text.starts_with("LEFT") {
            crate::velesql::JoinType::Left
        } else if text.starts_with("RIGHT") {
            crate::velesql::JoinType::Right
        } else if text.starts_with("FULL") {
            crate::velesql::JoinType::Full
        } else {
            crate::velesql::JoinType::Inner
        }
    }

    /// Parse JOIN condition (table.column = var.property).
    pub(crate) fn parse_join_condition(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<JoinCondition, ParseError> {
        let mut refs: Vec<ColumnRef> = Vec::new();

        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::column_ref {
                refs.push(Self::parse_column_ref(&inner_pair)?);
            }
        }

        if refs.len() != 2 {
            return Err(ParseError::syntax(
                0,
                "",
                "JOIN condition requires exactly two column references",
            ));
        }

        // SAFETY: refs.len() == 2 is validated above, pop() cannot fail
        let right = refs.pop().expect("right ref validated by len check");
        let left = refs.pop().expect("left ref validated by len check");

        Ok(JoinCondition { left, right })
    }

    /// Parse column reference (table.column).
    pub(crate) fn parse_column_ref(
        pair: &pest::iterators::Pair<Rule>,
    ) -> Result<ColumnRef, ParseError> {
        // column_ref is atomic (@), format: "table.column"
        let s = pair.as_str();
        let parts: Vec<&str> = s.split('.').collect();

        if parts.len() != 2 {
            return Err(ParseError::syntax(
                0,
                s,
                "Column reference must be in format 'table.column'",
            ));
        }

        Ok(ColumnRef {
            table: Some(parts[0].to_string()),
            column: parts[1].to_string(),
        })
    }

    /// Parse GROUP BY clause.
    pub(crate) fn parse_group_by_clause(pair: pest::iterators::Pair<Rule>) -> GroupByClause {
        let mut columns = Vec::new();

        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::group_by_list {
                for col_pair in inner_pair.into_inner() {
                    if col_pair.as_rule() == Rule::identifier {
                        columns.push(col_pair.as_str().to_string());
                    }
                }
            }
        }

        GroupByClause { columns }
    }

    /// Parse HAVING clause.
    pub(crate) fn parse_having_clause(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<HavingClause, ParseError> {
        let mut conditions = Vec::new();
        let mut operators = Vec::new();

        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::having_condition {
                for term_pair in inner_pair.into_inner() {
                    match term_pair.as_rule() {
                        Rule::having_term => {
                            conditions.push(Self::parse_having_term(term_pair)?);
                        }
                        _ => {
                            // Check for AND/OR keywords
                            let text = term_pair.as_str().to_uppercase();
                            if text == "AND" {
                                operators.push(crate::velesql::LogicalOp::And);
                            } else if text == "OR" {
                                operators.push(crate::velesql::LogicalOp::Or);
                            }
                        }
                    }
                }
            }
        }

        Ok(HavingClause {
            conditions,
            operators,
        })
    }

    /// Parse a single HAVING term (aggregate op value).
    fn parse_having_term(pair: pest::iterators::Pair<Rule>) -> Result<HavingCondition, ParseError> {
        let mut aggregate = None;
        let mut operator = None;
        let mut value = None;

        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::aggregate_function => {
                    aggregate = Some(Self::parse_aggregate_function_only(inner_pair)?);
                }
                Rule::compare_op => {
                    operator = Some(Self::parse_compare_op(&inner_pair)?);
                }
                Rule::value => {
                    value = Some(Self::parse_value(inner_pair)?);
                }
                _ => {}
            }
        }

        Ok(HavingCondition {
            aggregate: aggregate
                .ok_or_else(|| ParseError::syntax(0, "", "HAVING requires aggregate function"))?,
            operator: operator
                .ok_or_else(|| ParseError::syntax(0, "", "HAVING requires comparison operator"))?,
            value: value.ok_or_else(|| ParseError::syntax(0, "", "HAVING requires value"))?,
        })
    }

    /// Parse aggregate function for HAVING (returns AggregateFunction).
    fn parse_aggregate_function_only(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<AggregateFunction, ParseError> {
        let (function_type, argument) = Self::parse_aggregate_function(pair)?;
        Ok(AggregateFunction {
            function_type,
            argument,
            alias: None,
        })
    }

    /// Parse comparison operator.
    fn parse_compare_op(pair: &pest::iterators::Pair<Rule>) -> Result<CompareOp, ParseError> {
        match pair.as_str() {
            "=" => Ok(CompareOp::Eq),
            "!=" | "<>" => Ok(CompareOp::NotEq),
            ">" => Ok(CompareOp::Gt),
            ">=" => Ok(CompareOp::Gte),
            "<" => Ok(CompareOp::Lt),
            "<=" => Ok(CompareOp::Lte),
            other => Err(ParseError::syntax(0, other, "Unknown comparison operator")),
        }
    }

    /// Parse USING FUSION clause (EPIC-040 US-005).
    fn parse_using_fusion_clause(
        pair: pest::iterators::Pair<Rule>,
    ) -> crate::velesql::FusionClause {
        let mut strategy = crate::velesql::FusionStrategyType::Rrf;
        let mut k = None;
        let mut vector_weight = None;
        let mut graph_weight = None;

        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::fusion_options {
                for opt_pair in inner_pair.into_inner() {
                    if opt_pair.as_rule() == Rule::fusion_option_list {
                        for option in opt_pair.into_inner() {
                            if option.as_rule() == Rule::fusion_option {
                                let mut key = String::new();
                                let mut value_str = String::new();

                                for part in option.into_inner() {
                                    match part.as_rule() {
                                        Rule::identifier => key = part.as_str().to_lowercase(),
                                        Rule::fusion_value => {
                                            value_str =
                                                part.as_str().trim_matches('\'').to_string();
                                        }
                                        _ => {}
                                    }
                                }

                                match key.as_str() {
                                    "strategy" => {
                                        strategy = match value_str.to_lowercase().as_str() {
                                            "weighted" => {
                                                crate::velesql::FusionStrategyType::Weighted
                                            }
                                            "maximum" => {
                                                crate::velesql::FusionStrategyType::Maximum
                                            }
                                            _ => crate::velesql::FusionStrategyType::Rrf, // rrf is default
                                        };
                                    }
                                    "k" => {
                                        k = value_str.parse().ok();
                                    }
                                    "vector_weight" => {
                                        vector_weight = value_str.parse().ok();
                                    }
                                    "graph_weight" => {
                                        graph_weight = value_str.parse().ok();
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }

        crate::velesql::FusionClause {
            strategy,
            k,
            vector_weight,
            graph_weight,
        }
    }
}
