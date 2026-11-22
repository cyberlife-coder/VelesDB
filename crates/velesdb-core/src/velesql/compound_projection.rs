//! Positional outputs which cannot be represented as one input document.

use rustc_hash::FxHashSet;
use serde_json::Value;

use super::json_path::ProjectedField;
use super::window_evaluator::WindowInputs;
use super::{
    is_builtin_score_variable, ArithmeticExpr, Column, OrderByExpr, Query, SelectColumns,
    SelectOrderBy, SimilarityScoreExpr, WindowFunction,
};
use crate::SearchResult;

pub(super) struct CompoundProjection<'a> {
    // Hidden positions still participate in the operand's set key.
    named: Vec<Option<(&'a str, ProjectedField<'a>)>>,
    // Only a value stored away from its positional source can shadow
    // an unrelated named vector. A realigned source is the vector to rank.
    auxiliary: Vec<&'a str>,
    positional_vectors: Vec<(&'a str, usize)>,
}

#[path = "compound_sort_slots.rs"]
mod sort_slots;
pub(super) use sort_slots::{
    exchange_member, orders_named_vector, reads_vector_input, VectorSortSlots,
};
use sort_slots::{has_positional_vector, retains_vector_input, source_within};

struct VisibleColumn<'a> {
    position: usize,
    source: &'a str,
    output: &'a str,
}

impl VisibleColumn<'_> {
    fn reuses_member(&self) -> bool {
        self.source == self.output && self.source != "id" && !self.source.contains('.')
    }

    fn reads_point_id(&self, query: &Query) -> bool {
        if self.source != "id" {
            return false;
        }
        if !qualified_wildcard(&query.select.columns) {
            return true;
        }
        let (columns, _) = super::set_operation::select_fields(&query.select.columns);
        columns
            .iter()
            .any(|col| source_within(&col.name, self.output))
            && query
                .compound
                .iter()
                .flat_map(|c| &c.operations)
                .all(|(_, operand)| {
                    super::set_operation::select_fields(&operand.columns)
                        .0
                        .get(self.position)
                        .is_some_and(|col| col.name == "id")
                })
    }
}

/// A repeated source, or a selected path below another selected source, needs
/// independent storage for the values supplied by later operands.
fn sources_overlap(columns: &[Column]) -> bool {
    let sources: FxHashSet<&str> = columns.iter().map(|col| col.name.as_str()).collect();
    let nested = |col: &Column| {
        col.name
            .match_indices('.')
            .any(|(end, _)| sources.contains(&col.name[..end]))
    };
    sources.len() < columns.len() || columns.iter().any(nested)
}

fn computed_outputs<'a>(
    windows: &'a [WindowFunction],
    scores: &'a [SimilarityScoreExpr],
) -> FxHashSet<&'a str> {
    windows
        .iter()
        .map(|wf| {
            wf.alias
                .as_deref()
                .unwrap_or(wf.function_type.default_alias())
        })
        .chain(
            scores
                .iter()
                .map(|s| s.alias.as_deref().unwrap_or("similarity")),
        )
        .collect()
}

impl<'a> CompoundProjection<'a> {
    pub(super) fn for_query(query: &'a Query) -> Option<Self> {
        query.compound.as_ref()?;
        let (columns, windows, scores) = match &query.select.columns {
            SelectColumns::Columns(cols) => (cols.as_slice(), &[][..], &[][..]),
            SelectColumns::Mixed {
                columns,
                window_functions,
                similarity_scores,
                ..
            } => (
                columns.as_slice(),
                window_functions.as_slice(),
                similarity_scores.as_slice(),
            ),
            _ => return None,
        };
        let rank_inputs = compound_rank_inputs(query, &query.select.columns);
        let published =
            qualified_wildcard(&query.select.columns) || window_publishes_columns(columns, windows);
        needs_positional_slots(query, columns, published, &rank_inputs).then(|| {
            let mut layout = Self::from_sources(query, columns, windows, scores, published);
            if !published && rank_inputs.is_empty() {
                layout.preserve_disjoint_paths(columns);
            }
            layout.bind_ranks(windows, &rank_inputs);
            layout.bind_vector_sources(columns, query);
            layout
        })
    }

    // The columns-only projector follows nested source paths. Preserve those
    // paths when the layout exists solely to realign another operand's vector.
    fn preserve_disjoint_paths(&mut self, columns: &'a [Column]) {
        if sources_overlap(columns) {
            return;
        }
        for (binding, col) in self.named.iter_mut().zip(columns) {
            if let Some((_, field)) = binding {
                if *field == ProjectedField::Published(&col.name) && col.name.contains('.') {
                    *field = ProjectedField::Source(&col.name);
                }
            }
        }
    }

    fn bind_ranks(&mut self, windows: &'a [WindowFunction], inputs: &[String]) {
        if inputs.is_empty() {
            return;
        }
        for wf in windows {
            let name = wf
                .alias
                .as_deref()
                .unwrap_or(wf.function_type.default_alias());
            self.named
                .push(Some((name, ProjectedField::Published(name))));
            if inputs.iter().any(|n| n == name) {
                self.auxiliary.push(name);
            }
        }
    }

    /// When published aliases are the output slots, a vector selected under
    /// another name can still receive a different source by operand position.
    fn bind_vector_sources(&mut self, columns: &'a [Column], query: &Query) {
        for item in super::compound_window(query).order_by.unwrap_or_default() {
            let OrderByExpr::Similarity(sim) = &item.expr else {
                continue;
            };
            if sim.field == super::PRIMARY_VECTOR_FIELD {
                continue;
            }
            let selected = columns
                .iter()
                .enumerate()
                .filter(|(i, col)| col.name == sim.field && self.named[*i].is_some());
            let direct = selected
                .clone()
                .find(|(i, col)| self.named[*i].is_some_and(|(_, slot)| slot.name() == col.name));
            if let Some((position, column)) = direct.or_else(|| selected.into_iter().next()) {
                self.bind_positional_vector(query, &column.name, position, direct.is_some());
            }
        }
    }

    fn bind_positional_vector(
        &mut self,
        query: &Query,
        name: &'a str,
        position: usize,
        direct: bool,
    ) {
        let other_source = query
            .compound
            .iter()
            .flat_map(|c| &c.operations)
            .any(|(_, operand)| {
                super::set_operation::select_fields(&operand.columns)
                    .0
                    .get(position)
                    .is_some_and(|col| col.name != name)
            });
        if other_source {
            self.positional_vectors.push((name, position));
            if direct && !self.auxiliary.contains(&name) {
                self.auxiliary.push(name);
            }
        }
    }

    fn from_sources(
        query: &Query,
        columns: &'a [Column],
        windows: &'a [WindowFunction],
        scores: &'a [SimilarityScoreExpr],
        published: bool,
    ) -> Self {
        let computed = computed_outputs(windows, scores);
        // The projector writes columns in SELECT order, then writes computed
        // outputs. Only the last write to each JSON output name is observable.
        let mut seen = computed.clone();
        let visible: Vec<_> = columns
            .iter()
            .enumerate()
            .rev()
            .filter_map(|(position, col)| {
                let output = col.alias.as_deref().unwrap_or(&col.name);
                seen.insert(output).then_some(VisibleColumn {
                    position,
                    source: &col.name,
                    output,
                })
            })
            .collect();
        let mut layout = Self {
            named: vec![None; columns.len()],
            auxiliary: Vec::new(),
            positional_vectors: Vec::new(),
        };
        if published {
            layout.bind_published(&visible, query);
            return layout;
        }
        let mut used = computed;
        for kind in OrderName::BY_RESTRICTION {
            let group: Vec<_> = visible
                .iter()
                .rev()
                .filter(|col| OrderName::of(col.output) == kind)
                .collect();
            layout.bind_slots(&group, &mut used, kind);
        }
        layout
    }

    /// The public SELECT projector reads a captured window's columns by their
    /// output names, even when its caller only supplies the first SELECT.
    fn bind_published(&mut self, visible: &[VisibleColumn<'a>], query: &Query) {
        for col in visible {
            let field = if col.reads_point_id(query) {
                ProjectedField::Source("id")
            } else {
                if !col.reuses_member() {
                    self.auxiliary.push(col.output);
                }
                ProjectedField::Published(col.output)
            };
            self.named[col.position] = Some((col.output, field));
        }
    }

    fn bind_slots(
        &mut self,
        group: &[&VisibleColumn<'a>],
        used: &mut FxHashSet<&'a str>,
        kind: OrderName,
    ) {
        let mut pending = Vec::new();
        for col in group {
            let field = if col.source == "id" {
                ProjectedField::Source("id")
            } else if OrderName::of(col.source) <= kind && used.insert(col.source) {
                ProjectedField::Published(col.source)
            } else {
                pending.push(*col);
                continue;
            };
            self.named[col.position] = Some((col.output, field));
        }
        // The group's distinct output names provide one slot per column.
        // Each successful source claim occupies at most one of those slots.
        let free: Vec<_> = group
            .iter()
            .map(|col| col.output)
            .filter(|name| !used.contains(name))
            .collect();
        for (col, slot) in pending.into_iter().zip(free) {
            used.insert(slot);
            self.named[col.position] = Some((col.output, ProjectedField::Published(slot)));
            if col.source != slot {
                self.auxiliary.push(slot);
            }
        }
    }

    pub(super) fn destination(
        &self,
        position: usize,
        fallback: ProjectedField<'a>,
    ) -> Option<ProjectedField<'a>> {
        self.named
            .get(position)
            .map_or(Some(fallback), |binding| binding.map(|(_, slot)| slot))
    }

    pub(super) fn positional_vectors(&self) -> &[(&'a str, usize)] {
        &self.positional_vectors
    }

    pub(super) fn order(&self, items: &[SelectOrderBy]) -> Vec<SelectOrderBy> {
        let mut mapped = items.to_vec();
        for item in &mut mapped {
            match &mut item.expr {
                OrderByExpr::Field(name) => self.bind_order_name(name, OrderName::System),
                OrderByExpr::Arithmetic(expr) => self.bind_arithmetic(expr),
                _ => {}
            }
        }
        mapped
    }

    fn bind_order_name(&self, name: &mut String, reserved_from: OrderName) {
        if OrderName::of(name) >= reserved_from {
            return;
        }
        if let Some((_, ProjectedField::Source(slot) | ProjectedField::Published(slot))) = self
            .named
            .iter()
            .flatten()
            .find(|(output, _)| *output == name)
        {
            *name = (*slot).to_owned();
        }
    }

    fn bind_arithmetic(&self, expr: &mut ArithmeticExpr) {
        let mut pending = vec![expr];
        while let Some(node) = pending.pop() {
            match node {
                ArithmeticExpr::Variable(name) => {
                    self.bind_order_name(name, OrderName::ArithmeticScore);
                }
                ArithmeticExpr::BinaryOp { left, right, .. } => {
                    pending.extend([left.as_mut(), right.as_mut()]);
                }
                ArithmeticExpr::Literal(_) | ArithmeticExpr::Similarity(_) => {}
            }
        }
    }

    fn apply(&self, row: &SearchResult, output: &mut Value) {
        let Some(map) = output.as_object_mut() else {
            return;
        };
        for &(name, field) in self.named.iter().flatten() {
            if let ProjectedField::Published(_) = field {
                map.insert(name.to_owned(), field.read(row).into_owned());
            }
        }
    }
}

/// Source fields retained alongside a compound operand's window ranks. A
/// named-vector order can need a shadowed input. A qualified wildcard also
/// needs the original members when aligning the public output names of its
/// operand with the first SELECT. Compute this list before running the operand.
#[must_use]
pub fn compound_rank_inputs(query: &Query, columns: &SelectColumns) -> Vec<String> {
    let SelectColumns::Mixed {
        window_functions, ..
    } = columns
    else {
        return Vec::new();
    };
    if query.compound.is_none() || window_functions.is_empty() {
        return Vec::new();
    }
    let order = super::compound_window(query).order_by.unwrap_or_default();
    let mut names = Vec::new();
    for (position, wf) in window_functions.iter().enumerate() {
        let name = wf
            .alias
            .as_deref()
            .unwrap_or(wf.function_type.default_alias());
        let retained = retains_vector_input(query, columns, order, name)
            || wildcard_retains_input(query, columns, name, position);
        if retained && !names.iter().any(|n| n == name) {
            names.push(name.to_owned());
        }
    }
    names
}

fn wildcard_retains_input(
    query: &Query,
    selection: &SelectColumns,
    name: &str,
    index: usize,
) -> bool {
    if !qualified_wildcard(selection) {
        return false;
    }
    let (columns, _) = super::set_operation::select_fields(selection);
    let (_, first) = super::set_operation::select_fields(&query.select.columns);
    let projected_path = columns.iter().any(|col| {
        col.name
            .split_once('.')
            .is_some_and(|(root, _)| root == name)
    });
    projected_path
        || first.get(index).is_some_and(|wf| {
            wf.alias
                .as_deref()
                .unwrap_or(wf.function_type.default_alias())
                != name
        })
}

fn occupy_order_names<'a>(expr: &'a OrderByExpr, occupied: &mut FxHashSet<&'a str>) {
    match expr {
        OrderByExpr::Field(name) => {
            occupied.insert(name);
        }
        OrderByExpr::Similarity(sim) => {
            occupied.insert(&sim.field);
        }
        OrderByExpr::Arithmetic(arithmetic) => {
            let mut pending = vec![arithmetic];
            while let Some(node) = pending.pop() {
                match node {
                    ArithmeticExpr::Variable(name) => {
                        occupied.insert(name);
                    }
                    ArithmeticExpr::BinaryOp { left, right, .. } => {
                        pending.extend([left.as_ref(), right.as_ref()]);
                    }
                    ArithmeticExpr::Similarity(order) => occupy_order_names(order, occupied),
                    ArithmeticExpr::Literal(_) => {}
                }
            }
        }
        OrderByExpr::Aggregate(_) | OrderByExpr::SimilarityBare => {}
    }
}

// A slot may stand for an output if every context that reads that output as a
// payload field also reads the slot as one. Bare similarity is a field, while
// ScoreContext::resolve_variable reads arithmetic similarity as the score.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum OrderName {
    Field,
    ArithmeticScore,
    System,
}

impl OrderName {
    const BY_RESTRICTION: [Self; 3] = [Self::Field, Self::ArithmeticScore, Self::System];

    fn of(name: &str) -> Self {
        if name == "id" || is_builtin_score_variable(name) {
            Self::System
        } else if name == "similarity" {
            Self::ArithmeticScore
        } else {
            Self::Field
        }
    }
}

pub(super) fn qualified_wildcard(columns: &SelectColumns) -> bool {
    matches!(columns, SelectColumns::Mixed { qualified_wildcards, .. } if !qualified_wildcards.is_empty())
}

fn needs_positional_slots(
    query: &Query,
    columns: &[Column],
    published: bool,
    ranks: &[String],
) -> bool {
    if !ranks.is_empty() {
        return true;
    }
    if qualified_wildcard(&query.select.columns) {
        return !columns.is_empty();
    }
    if published {
        return orders_named_vector(query);
    }
    (columns.len() >= 2 && sources_overlap(columns)) || has_positional_vector(query)
}

fn window_publishes_columns(columns: &[Column], windows: &[WindowFunction]) -> bool {
    !windows.is_empty()
        && WindowInputs::projection_fields(columns, windows)
            .any(|(_, field)| matches!(field, ProjectedField::Published(_)))
}

/// A windowed operand publishes captured columns by source. Positional
/// alignment assigns output slots once its set key and vector inputs are known.
pub(super) fn preserve_operand_positions(selection: &mut SelectColumns, vector_order: bool) {
    if qualified_wildcard(selection) {
        return;
    }
    let SelectColumns::Mixed {
        columns,
        window_functions,
        ..
    } = selection
    else {
        return;
    };
    if window_publishes_columns(columns, window_functions)
        && (vector_order || window_output_repeats(columns, window_functions))
    {
        for col in columns {
            col.alias = None;
        }
    }
}

fn window_output_repeats(columns: &[Column], window_functions: &[WindowFunction]) -> bool {
    let mut outputs: FxHashSet<_> = window_functions
        .iter()
        .map(|wf| {
            wf.alias
                .as_deref()
                .unwrap_or(wf.function_type.default_alias())
        })
        .collect();
    columns
        .iter()
        .any(|col| !outputs.insert(col.alias.as_deref().unwrap_or(&col.name)))
}

/// Project executed query rows with the first operand's positional output
/// schema for a compound, and the ordinary SELECT projection otherwise.
pub fn project_query_rows<'a>(
    query: &'a Query,
    rows: &'a [SearchResult],
    mut project: impl FnMut(&SearchResult, &SelectColumns) -> Value + 'a,
) -> impl Iterator<Item = (&'a SearchResult, Value)> + 'a {
    let layout = CompoundProjection::for_query(query);
    rows.iter().map(move |row| {
        let mut output = project(row, &query.select.columns);
        if let Some(layout) = &layout {
            layout.apply(row, &mut output);
        }
        (row, output)
    })
}
