//! Specialized condition parsers: similarity, contains, and geo expressions.
//!
//! These leaf-level condition parsers are separated from the core condition
//! dispatch tree (`conditions.rs`) to keep file NLOC under 500.

use super::helpers::compare_op_from_str;
use super::Rule;
use crate::velesql::ast::{
    Condition, ContainsCondition, ContainsMode, GeoBboxCondition, GeoDistanceCondition,
    SimilarityCondition,
};
use crate::velesql::error::ParseError;
use crate::velesql::Parser;

impl Parser {
    /// Parses a similarity expression: `similarity(field, vector) op threshold`
    pub(crate) fn parse_similarity_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        let mut field = None;
        let mut vector = None;
        let mut operator = None;
        let mut threshold = None;

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::similarity_field => {
                    field = Some(inner.as_str().to_string());
                }
                Rule::vector_value => {
                    vector = Some(Self::parse_vector_value(inner)?);
                }
                Rule::compare_op => {
                    operator = Some(compare_op_from_str(inner.as_str())?);
                }
                Rule::numeric_threshold => {
                    // numeric_threshold = { float | integer }
                    let inner_value = inner
                        .into_inner()
                        .next()
                        .ok_or_else(|| ParseError::syntax(0, "", "Expected numeric threshold"))?;
                    threshold = Some(inner_value.as_str().parse::<f64>().map_err(|_| {
                        ParseError::syntax(0, inner_value.as_str(), "Invalid threshold")
                    })?);
                }
                _ => {}
            }
        }

        let field = field.ok_or_else(|| ParseError::syntax(0, "", "Expected field name"))?;
        let vector =
            vector.ok_or_else(|| ParseError::syntax(0, "", "Expected vector expression"))?;
        let operator = operator.ok_or_else(|| ParseError::syntax(0, "", "Expected operator"))?;
        let threshold =
            threshold.ok_or_else(|| ParseError::syntax(0, "", "Expected threshold value"))?;

        Ok(Condition::Similarity(SimilarityCondition {
            field,
            vector,
            operator,
            threshold,
        }))
    }

    /// Parses a CONTAINS expression into a `Condition::Contains`.
    ///
    /// Handles three forms:
    /// - `column CONTAINS ALL (v1, v2, ...)`
    /// - `column CONTAINS ANY (v1, v2, ...)`
    /// - `column CONTAINS value`
    pub(crate) fn parse_contains_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        let raw = pair.as_str();
        let mut inner = pair.into_inner();
        let column = Self::extract_leading_column(&mut inner)?;

        let (mode, values) = Self::detect_contains_mode(&mut inner, raw)?;

        Ok(Condition::Contains(ContainsCondition {
            column,
            mode,
            values,
        }))
    }

    /// Detects the CONTAINS mode (ALL, ANY, or Single) and parses values.
    fn detect_contains_mode(
        inner: &mut pest::iterators::Pairs<Rule>,
        raw: &str,
    ) -> Result<(ContainsMode, Vec<crate::velesql::ast::Value>), ParseError> {
        let upper = raw.to_uppercase();
        if upper.contains("CONTAINS ALL") {
            let values = Self::collect_value_list(inner, raw)?;
            Ok((ContainsMode::All, values))
        } else if upper.contains("CONTAINS ANY") {
            let values = Self::collect_value_list(inner, raw)?;
            Ok((ContainsMode::Any, values))
        } else {
            let value = Self::next_value(inner, "Expected value after CONTAINS")?;
            Ok((ContainsMode::Single, vec![value]))
        }
    }

    /// Collects values from a `value_list` rule in the parse tree.
    fn collect_value_list(
        inner: &mut pest::iterators::Pairs<Rule>,
        raw: &str,
    ) -> Result<Vec<crate::velesql::ast::Value>, ParseError> {
        let value_list = inner
            .find(|p| p.as_rule() == Rule::value_list)
            .ok_or_else(|| ParseError::syntax(0, raw, "Expected value list"))?;

        value_list
            .into_inner()
            .filter(|p| p.as_rule() == Rule::value)
            .map(Self::parse_value)
            .collect()
    }

    /// Parses a `geo_number` rule into an `f64`.
    fn parse_geo_number(pair: pest::iterators::Pair<Rule>) -> Result<f64, ParseError> {
        let inner = pair
            .into_inner()
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected numeric value"))?;
        inner
            .as_str()
            .parse::<f64>()
            .map_err(|_| ParseError::syntax(0, inner.as_str(), "Invalid numeric value"))
    }

    /// Parses a `GEO_DISTANCE(column, lat, lng) op threshold` expression.
    pub(crate) fn parse_geo_distance_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        let mut inner = pair.into_inner();
        let column_pair = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected column name"))?;
        let column = Self::extract_column_name(&column_pair);

        let lat = Self::parse_geo_number(
            inner
                .next()
                .ok_or_else(|| ParseError::syntax(0, "", "Expected latitude"))?,
        )?;
        let lng = Self::parse_geo_number(
            inner
                .next()
                .ok_or_else(|| ParseError::syntax(0, "", "Expected longitude"))?,
        )?;

        let op_pair = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected comparison operator"))?;
        let operator = compare_op_from_str(op_pair.as_str())?;

        let threshold = Self::parse_geo_number(
            inner
                .next()
                .ok_or_else(|| ParseError::syntax(0, "", "Expected distance threshold"))?,
        )?;

        Ok(Condition::GeoDistance(GeoDistanceCondition {
            column,
            lat,
            lng,
            operator,
            threshold,
        }))
    }

    /// Parses a `GEO_BBOX(column, lat_min, lng_min, lat_max, lng_max)` expression.
    pub(crate) fn parse_geo_bbox_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        let mut inner = pair.into_inner();
        let column_pair = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected column name"))?;
        let column = Self::extract_column_name(&column_pair);

        let lat_min = Self::parse_geo_number(
            inner
                .next()
                .ok_or_else(|| ParseError::syntax(0, "", "Expected lat_min"))?,
        )?;
        let lng_min = Self::parse_geo_number(
            inner
                .next()
                .ok_or_else(|| ParseError::syntax(0, "", "Expected lng_min"))?,
        )?;
        let lat_max = Self::parse_geo_number(
            inner
                .next()
                .ok_or_else(|| ParseError::syntax(0, "", "Expected lat_max"))?,
        )?;
        let lng_max = Self::parse_geo_number(
            inner
                .next()
                .ok_or_else(|| ParseError::syntax(0, "", "Expected lng_max"))?,
        )?;

        Ok(Condition::GeoBbox(GeoBboxCondition {
            column,
            lat_min,
            lng_min,
            lat_max,
            lng_max,
        }))
    }
}
