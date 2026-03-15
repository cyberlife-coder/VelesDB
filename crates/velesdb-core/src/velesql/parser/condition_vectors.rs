//! Vector-related condition parsing helpers (dense, sparse, fused).

use super::Rule;
use crate::sparse_index::SparseVector;
use crate::velesql::ast::condition::{SparseVectorExpr, SparseVectorSearch};
use crate::velesql::ast::{Condition, FusionConfig, VectorExpr, VectorFusedSearch, VectorSearch};
use crate::velesql::error::ParseError;
use crate::velesql::Parser;

impl Parser {
    pub(crate) fn parse_vector_search(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        let mut vector = None;

        for inner in pair.into_inner() {
            if inner.as_rule() == Rule::vector_value {
                vector = Some(Self::parse_vector_value(inner)?);
            }
        }

        let vector =
            vector.ok_or_else(|| ParseError::syntax(0, "", "Expected vector expression"))?;

        Ok(Condition::VectorSearch(VectorSearch { vector }))
    }

    /// Parses a sparse vector search: `vector SPARSE_NEAR sparse_value [USING 'index-name']`
    pub(crate) fn parse_sparse_vector_search(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        let mut vector = None;
        let mut index_name = None;

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::sparse_value => {
                    vector = Some(Self::parse_sparse_value(inner)?);
                }
                Rule::string => {
                    // USING clause: string value is the index name.
                    //
                    // The `string` grammar rule is defined as an atomic rule:
                    //   string = @{ "'" ~ (!"'" ~ ANY)* ~ "'" }
                    // `pair.as_str()` therefore includes the surrounding single
                    // quotes (e.g. `'my-index'`), so `trim_matches('\'')` is
                    // required and correct — it is not a no-op.
                    //
                    // Only single-quoted strings are accepted by the grammar.
                    // Double-quoted identifiers (e.g. `USING "body"`) are NOT
                    // supported and will fail at the pest parse stage before
                    // reaching this branch.
                    index_name = Some(inner.as_str().trim_matches('\'').to_string());
                }
                _ => {}
            }
        }

        let vector =
            vector.ok_or_else(|| ParseError::syntax(0, "", "Expected sparse vector expression"))?;

        Ok(Condition::SparseVectorSearch(SparseVectorSearch {
            vector,
            index_name,
        }))
    }

    /// Parses a single sparse entry `index: value` into an `(u32, f32)` pair.
    fn parse_sparse_entry(entry: pest::iterators::Pair<Rule>) -> Result<(u32, f32), ParseError> {
        let mut entry_inner = entry.into_inner();
        let idx = entry_inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected sparse entry index"))?
            .as_str()
            .parse::<u32>()
            .map_err(|_| ParseError::syntax(0, "", "Invalid sparse entry index"))?;
        let val = entry_inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected sparse entry value"))?
            .as_str()
            .parse::<f32>()
            .map_err(|_| ParseError::syntax(0, "", "Invalid sparse entry value"))?;
        Ok((idx, val))
    }

    /// Parses a sparse literal `{12: 0.8, 45: 0.3}` into a `SparseVector`.
    fn parse_sparse_literal(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<SparseVectorExpr, ParseError> {
        let mut pairs = Vec::new();
        for entry in pair.into_inner() {
            if entry.as_rule() == Rule::sparse_entry {
                pairs.push(Self::parse_sparse_entry(entry)?);
            }
        }
        Ok(SparseVectorExpr::Literal(SparseVector::new(pairs)))
    }

    /// Parses a sparse value: either a sparse literal `{12: 0.8, 45: 0.3}` or a parameter `$sv`.
    fn parse_sparse_value(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<SparseVectorExpr, ParseError> {
        let inner = pair
            .into_inner()
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected sparse vector expression"))?;

        match inner.as_rule() {
            Rule::sparse_literal => Self::parse_sparse_literal(inner),
            Rule::parameter => {
                let name = inner.as_str().trim_start_matches('$').to_string();
                Ok(SparseVectorExpr::Parameter(name))
            }
            _ => Err(ParseError::syntax(
                0,
                inner.as_str(),
                "Expected sparse literal or parameter",
            )),
        }
    }

    pub(crate) fn parse_vector_fused_search(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        let mut vectors = Vec::new();
        let mut fusion = FusionConfig::default();

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::vector_array => {
                    for vec_value in inner.into_inner() {
                        if vec_value.as_rule() == Rule::vector_value {
                            vectors.push(Self::parse_vector_value(vec_value)?);
                        }
                    }
                }
                Rule::fusion_clause => {
                    fusion = Self::parse_fusion_clause(inner);
                }
                _ => {}
            }
        }

        if vectors.is_empty() {
            return Err(ParseError::syntax(
                0,
                "",
                "Expected at least one vector in NEAR_FUSED",
            ));
        }

        Ok(Condition::VectorFusedSearch(VectorFusedSearch {
            vectors,
            fusion,
        }))
    }

    /// Extracts key-value fusion parameters from a `fusion_params` rule.
    fn extract_fusion_params(
        pair: pest::iterators::Pair<Rule>,
    ) -> std::collections::HashMap<String, f64> {
        let mut params = std::collections::HashMap::new();
        for param in pair.into_inner() {
            if param.as_rule() == Rule::fusion_param_list {
                for fp in param.into_inner() {
                    if fp.as_rule() == Rule::fusion_param {
                        let mut fp_inner = fp.into_inner();
                        if let (Some(key), Some(val)) = (fp_inner.next(), fp_inner.next()) {
                            let key_str = key.as_str().to_string();
                            let val_f64 = val.as_str().parse::<f64>().unwrap_or(0.0);
                            params.insert(key_str, val_f64);
                        }
                    }
                }
            }
        }
        params
    }

    pub(crate) fn parse_fusion_clause(pair: pest::iterators::Pair<Rule>) -> FusionConfig {
        let mut strategy = "rrf".to_string();
        let mut params = std::collections::HashMap::new();

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::fusion_strategy => {
                    strategy = inner.into_inner().next().map_or_else(
                        || "rrf".to_string(),
                        |s| s.as_str().trim_matches('\'').to_string(),
                    );
                }
                Rule::fusion_params => {
                    params = Self::extract_fusion_params(inner);
                }
                _ => {}
            }
        }

        FusionConfig { strategy, params }
    }

    pub(crate) fn parse_vector_value(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<VectorExpr, ParseError> {
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
}
