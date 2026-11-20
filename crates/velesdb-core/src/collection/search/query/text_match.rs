//! A WHERE's text `MATCH` leaves, bound to this collection's BM25 hits
//! before the WHERE is evaluated (`velesql::text_match`): one leaf may stay
//! the query's ranking, every other one becomes the `IN` list of its hits'
//! ids, read from the text index once per distinct query.
//!
//! LOCK ORDER: a binding reads the text index under its own locks, with no
//! collection lock held — every caller binds before it takes a storage
//! guard.

use std::borrow::Cow;
use std::sync::Arc;

use crate::collection::types::Collection;
use crate::error::{Error, Result};
use crate::velesql::text_match::{bind_text_matches, holds_text_match, TextRanking};
use crate::velesql::{Condition, InCondition, MatchClause, MatchCondition, Value};

/// One collection's hits for each text query a binding has read, so each
/// is read from the text index once, however many leaves ask for it
/// ([`Collection::text_match_membership`]). Shared, so a leaf's `IN` list
/// copies them once.
#[derive(Debug, Default)]
pub(crate) struct TextHits(Vec<(String, Arc<[Value]>)>);

impl TextHits {
    /// The hits of `query`, read through `ids` the first time it is asked.
    fn of(&mut self, query: &str, ids: impl FnOnce() -> Vec<u64>) -> Vec<Value> {
        if let Some((_, hits)) = self.0.iter().find(|(read, _)| read == query) {
            return hits.to_vec();
        }
        let hits: Arc<[Value]> = ids().into_iter().map(Value::from).collect();
        let values = hits.to_vec();
        self.0.push((query.to_string(), hits));
        values
    }
}

/// What evaluates a bound WHERE, which decides whether its ranking `MATCH`
/// stays one ([`TextRanking`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WhereReader {
    /// A SELECT: the ranking `MATCH` ranks its rows, and is left as written.
    Select,
    /// An aggregate ranks nothing: it counts the rows the WHERE selects. A
    /// hybrid's `MATCH`, a boost that selects nothing, is left as written
    /// (it reads TRUE); a text search's, which selects its hits, is bound.
    Aggregate,
}

/// The column a bound `MATCH` reads in a MATCH statement's WHERE: the id
/// of the node its alias binds (`a.content MATCH` → `a.id`), or of the
/// row's node for a column no alias qualifies — as every other WHERE leaf
/// resolves its column. A relationship has no text, and an annotated node
/// (`(a@other)`) reads another collection's, which this text index does not
/// hold: both are refused.
fn match_statement_id(clause: &MatchClause, leaf: &MatchCondition) -> Result<String> {
    let Some((alias, _)) = leaf.column.split_once('.') else {
        return Ok("id".to_string());
    };
    let named = |a: &Option<String>| a.as_deref() == Some(alias);
    let patterns = clause.patterns.iter();
    if patterns
        .clone()
        .flat_map(|p| &p.relationships)
        .any(|r| named(&r.alias))
    {
        return Err(Error::Query(format!(
            "a text MATCH reads a node's indexed text, and '{alias}' is a relationship"
        )));
    }
    match patterns.flat_map(|p| &p.nodes).find(|n| named(&n.alias)) {
        Some(node) => match node.collection.as_deref() {
            Some(other) => Err(Error::Query(format!(
                "a text MATCH reads the queried collection's text index, and '{alias}' \
                 reads @{other}"
            ))),
            None => Ok(format!("{alias}.id")),
        },
        None => Ok("id".to_string()),
    }
}

impl Collection {
    /// The ids this collection's text index scores for `query` — every
    /// document holding one of its terms, as `text_search` reads them —
    /// sorted, each once.
    pub(crate) fn text_match_ids(&self, query: &str) -> Vec<u64> {
        let mut ids: Vec<u64> = self
            .storage
            .text_index
            .score_all(query)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// The membership test of `leaf`: whether the row's id `column` reads is
    /// one of this collection's hits for its query — the `IN` list every
    /// binding of a `MATCH` writes, on one collection or a JOIN's sides.
    /// `hits` holds this collection's hits for each query read so far, so
    /// each distinct query is read from the text index once.
    pub(crate) fn text_match_membership(
        &self,
        leaf: &MatchCondition,
        column: String,
        hits: &mut TextHits,
    ) -> InCondition {
        InCondition {
            column,
            values: hits.of(&leaf.query, || self.text_match_ids(&leaf.query)),
            negated: false,
        }
    }

    /// `condition` with every `MATCH` but `keep` replaced by the `IN` list
    /// of this collection's hits for its query, on the column `column`
    /// names for it (a caller may refuse a `MATCH` it cannot read there)
    /// ([`Self::text_match_membership`]).
    ///
    /// # Errors
    ///
    /// What `column` returns.
    pub(crate) fn bind_text_matches<E>(
        &self,
        condition: &Condition,
        keep: Option<&MatchCondition>,
        column: &dyn Fn(&MatchCondition) -> std::result::Result<String, E>,
    ) -> std::result::Result<Condition, E> {
        let mut hits = TextHits::default();
        bind_text_matches(condition, keep, &mut |leaf| {
            Ok(self.text_match_membership(leaf, column(leaf)?, &mut hits))
        })
    }

    /// `condition` with every `MATCH` but `keep` bound to this collection's
    /// hits on the row's id: the WHERE of a SELECT, an aggregate, or a
    /// JOIN side's pushed filters.
    pub(crate) fn bind_row_text_matches(
        &self,
        condition: &Condition,
        keep: Option<&MatchCondition>,
    ) -> Condition {
        let Ok(bound) =
            self.bind_text_matches::<std::convert::Infallible>(condition, keep, &|_| {
                Ok("id".to_string())
            });
        bound
    }

    /// The WHERE `reader` evaluates on this collection's rows: `condition`
    /// with its ranking `MATCH` kept where `reader` ranks by it, every other
    /// `MATCH` bound to the ids of its hits.
    pub(crate) fn bind_where_text_matches(
        &self,
        condition: &Condition,
        reader: WhereReader,
    ) -> Condition {
        let keep = match (TextRanking::of(condition), reader) {
            (TextRanking::Text(leaf), WhereReader::Select) | (TextRanking::Hybrid(leaf), _) => {
                Some(leaf)
            }
            _ => None,
        };
        self.bind_row_text_matches(condition, keep)
    }

    /// `clause` with every text `MATCH` of its WHERE bound to this
    /// collection's hits ([`match_statement_id`] names the id each reads): a
    /// MATCH statement ranks no rows by text.
    ///
    /// # Errors
    ///
    /// A `MATCH` on a relationship or an annotated node.
    pub(crate) fn bind_match_statement_text<'c>(
        &self,
        clause: &'c MatchClause,
    ) -> Result<Cow<'c, MatchClause>> {
        let Some(condition) = clause.where_clause.as_ref().filter(|c| holds_text_match(c)) else {
            return Ok(Cow::Borrowed(clause));
        };
        let bound =
            self.bind_text_matches(condition, None, &|leaf| match_statement_id(clause, leaf))?;
        let mut clause = clause.clone();
        clause.where_clause = Some(bound);
        Ok(Cow::Owned(clause))
    }
}
