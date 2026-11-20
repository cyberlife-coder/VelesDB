//! A column qualified by the FROM side's name is the base row's own column.
//!
//! The FROM side is named by its alias, or by its table when it has none
//! ([`super::side_alias`]); an aliased table's own name names no side, as in
//! SQL. `d.cat` in `FROM docs d` is the row's `cat`, with or without a JOIN —
//! on a base row before any join, and on the rows a single-table SELECT or
//! aggregate reads, which carry the row's columns at their root. A qualifier
//! naming no side is left as written: on a single table it is the payload
//! path it spells (`meta.lang`, a nested field), in a JOIN a joined side's
//! column, read from the joined row (`velesql::merge_side`).
//!
//! One resolver for every reader of a base condition: the collection query
//! pipeline (`Collection::bind_query_where`, which a JOIN's base query runs
//! through too), the JOIN text-`MATCH` qualifier rule
//! (`Database::resolve_join_text_matches`) and the WASM executor.

use super::condition_tree::map_leaf_columns;
use super::{Condition, SelectStatement};

/// The name `select`'s FROM side goes by: its alias, else its table.
///
/// `from_alias` lists every alias in scope, the FROM's first when it has one,
/// then the JOINs' (`SelectStatement::from_alias`): its first entry is the
/// FROM's alias only when no JOIN is aliased by it. Reading the first entry
/// alone named an unaliased FROM after the first aliased JOIN — `FROM docs
/// JOIN tags t` scoped the base rows under `t`, which the join then
/// overwrote, so `docs.cat` read nothing.
#[must_use]
pub fn base_side(select: &SelectStatement) -> &str {
    let from_alias = select
        .from_alias
        .first()
        .map(String::as_str)
        .filter(|alias| {
            !select
                .joins
                .iter()
                .any(|join| join.alias.as_deref() == Some(alias))
        });
    super::side_alias(from_alias, &select.from)
}

/// `column` without its qualifier when that qualifier is `base`, the FROM
/// side's name; `None` for a column it does not qualify.
#[must_use]
pub(crate) fn base_column<'c>(column: &'c str, base: &str) -> Option<&'c str> {
    column
        .strip_prefix(base)
        .and_then(|rest| rest.strip_prefix('.'))
        .filter(|rest| !rest.is_empty())
}

/// `condition` with every column the FROM side's name `base` qualifies
/// written as the base row's own — `d.cat` as `cat` for `base` `d`; every
/// other column, and every leaf without one, as it is.
#[must_use]
pub fn resolve_base_qualifiers(condition: &Condition, base: &str) -> Condition {
    map_leaf_columns(condition, &|column| {
        base_column(column, base).map(str::to_string)
    })
}

#[cfg(test)]
#[path = "side_qualifier_tests.rs"]
mod tests;
