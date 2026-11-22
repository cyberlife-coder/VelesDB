//! Temporary fields used only while ordering a compound. Their names are
//! chosen after inspecting the input documents and the projected columns.
use super::{
    occupy_order_names, qualified_wildcard, Column, CompoundProjection, FxHashSet, OrderByExpr,
    ProjectedField, Query, SearchResult, SelectColumns, SelectOrderBy, Value,
};

/// Scalar values whose normal output slots shadow a vector input. The
/// alternative literal keys exist only while the compound is being ordered.
pub(in crate::velesql) struct VectorSortSlots {
    fields: Vec<DeferredMember>,
    written: Vec<String>,
}

struct DeferredMember {
    name: String,
    temporary: String,
    // These members can be absent when a different operand supplies the vector.
    original_at: Option<usize>,
}

impl VectorSortSlots {
    pub(in crate::velesql) fn publish(&self, page: &mut [SearchResult]) {
        let mut outputs = Vec::with_capacity(self.fields.len());
        let mut tail = Vec::new();
        for map in page
            .iter_mut()
            .filter_map(|row| row.point.payload.as_mut()?.as_object_mut())
        {
            outputs.extend(self.fields.iter().filter_map(|field| field.output(map)));
            if !outputs.is_empty() {
                self.publish_row(map, &mut outputs, &mut tail);
                outputs.clear();
            }
        }
    }

    fn publish_row(
        &self,
        map: &mut serde_json::Map<String, Value>,
        outputs: &mut [(&DeferredMember, Option<Value>)],
        tail: &mut Vec<(String, Value)>,
    ) {
        let mut in_tail = false;
        map.retain(|key, value| {
            if let Some((field, pending)) = outputs
                .iter_mut()
                .find(|(field, _)| key == &field.name || key == &field.temporary)
            {
                in_tail = in_tail || key == &field.temporary;
                return field.place(key, value, pending.take(), in_tail, tail);
            }
            if in_tail && self.written.iter().any(|name| name == key) {
                tail.push((key.clone(), value.take()));
                return false;
            }
            true
        });
        // Only alignment writes can follow a fresh slot in an insertion-ordered
        // map. Move that suffix while keeping unrelated user data in place.
        map.extend(tail.drain(..));
    }

    pub(in crate::velesql) fn replaced_members(&self, columns: &[Column]) -> Vec<(&str, &str)> {
        self.fields
            .iter()
            .filter_map(|field| {
                let col = columns.get(field.original_at?)?;
                (col.name != field.name).then_some((field.name.as_str(), field.temporary.as_str()))
            })
            .collect()
    }
}

impl DeferredMember {
    fn output(&self, map: &mut serde_json::Map<String, Value>) -> Option<(&Self, Option<Value>)> {
        let mut value = map.get_mut(&self.temporary)?.take();
        let output = if self.original_at.is_some() {
            value.as_array_mut().and_then(Vec::pop)
        } else {
            Some(value)
        };
        Some((self, output))
    }

    // A stored output keeps its original position. A new output takes its
    // SELECT position even when a later sort write created its public name.
    fn place(
        &self,
        key: &str,
        current: &mut Value,
        next: Option<Value>,
        in_tail: bool,
        tail: &mut Vec<(String, Value)>,
    ) -> bool {
        let Some(next) = next else {
            return false;
        };
        if !in_tail && key == self.name {
            *current = next;
            return true;
        }
        tail.push((self.name.clone(), next));
        false
    }
}

/// Exchange one literal member, preserving its position when it already exists.
#[inline(never)]
pub(in crate::velesql) fn exchange_member(
    payload: &mut Value,
    name: &str,
    replacement: Option<Value>,
) -> Option<Value> {
    let map = payload.as_object_mut()?;
    match (map.get_mut(name), replacement) {
        (Some(value), Some(new)) => Some(std::mem::replace(value, new)),
        (_, Some(new)) => map.insert(name.to_owned(), new),
        (_, None) => map.remove(name),
    }
}

fn written_member(field: ProjectedField<'_>) -> &str {
    match field {
        ProjectedField::Published(key) => key,
        ProjectedField::Source(path) => path.split_once('.').map_or(path, |(root, _)| root),
    }
}

impl<'a> CompoundProjection<'a> {
    /// A named vector reads its literal field first, then its dotted source
    /// path. Defer a write to either that literal or its root until the sort
    /// has read the original vector. A primary vector lives on the point.
    pub(in crate::velesql) fn vector_sort_slots<'r>(
        &self,
        order: &[SelectOrderBy],
        fields: impl Iterator<Item = ProjectedField<'a>>,
        rows: impl Iterator<Item = &'r SearchResult>,
        originals: Vec<(&str, usize)>,
    ) -> Option<VectorSortSlots> {
        let deferred = self.deferred_members(order, originals);
        if deferred.is_empty() {
            return None;
        }
        let mut occupied: FxHashSet<&str> = rows
            .filter_map(|row| row.point.payload.as_ref()?.as_object())
            .flat_map(|payload| payload.keys().map(String::as_str))
            .filter(|name| name.starts_with('.'))
            .collect();
        let mut written = FxHashSet::default();
        for &(output, field) in self.named.iter().flatten() {
            occupied.extend([output, field.name()]);
            written.insert(written_member(field));
        }
        for field in fields {
            occupied.insert(field.name());
            written.insert(written_member(field));
        }
        written.extend(self.positional_vectors.iter().map(|(name, _)| *name));
        for item in order {
            occupy_order_names(&item.expr, &mut occupied);
        }
        // A literal dotted key can shadow only a lookup of that exact name.
        let available = (0_usize..)
            .map(|i| format!(".{i}"))
            .filter(|k| !occupied.contains(k.as_str()));
        let fields = deferred
            .into_iter()
            .zip(available)
            .map(|((name, original_at), temporary)| DeferredMember {
                name: name.to_owned(),
                temporary,
                original_at,
            })
            .collect();
        Some(VectorSortSlots {
            fields,
            written: written.into_iter().map(str::to_owned).collect(),
        })
    }

    fn deferred_members<'r>(
        &self,
        order: &[SelectOrderBy],
        originals: Vec<(&'r str, usize)>,
    ) -> Vec<(&'r str, Option<usize>)>
    where
        'a: 'r,
    {
        let mut deferred: Vec<_> = originals
            .into_iter()
            .map(|(name, i)| (name, Some(i)))
            .collect();
        deferred.extend(
            self.auxiliary
                .iter()
                .copied()
                .filter(|slot| {
                    order
                        .iter()
                        .any(|item| reads_vector_input(&item.expr, slot))
                })
                .map(|slot| (slot, None)),
        );
        deferred
    }

    pub(in crate::velesql) fn redirect_slots(&mut self, slots: &'a VectorSortSlots) {
        for (_, field) in self.named.iter_mut().flatten() {
            if let ProjectedField::Published(name) = field {
                if let Some(slot) = slots
                    .fields
                    .iter()
                    .find(|slot| slot.original_at.is_none() && slot.name == *name)
                {
                    *field = ProjectedField::Published(&slot.temporary);
                }
            }
        }
    }

    pub(in crate::velesql) fn wildcard_replacements(&self, query: &Query) -> Vec<(&str, usize)> {
        if !qualified_wildcard(&query.select.columns) {
            return Vec::new();
        }
        self.positional_vectors
            .iter()
            .copied()
            .filter(|(name, _)| !self.auxiliary.contains(name))
            .collect()
    }
}

pub(in crate::velesql) fn reads_vector_input(expr: &OrderByExpr, slot: &str) -> bool {
    let OrderByExpr::Similarity(sim) = expr else {
        return false;
    };
    sim.field != super::super::PRIMARY_VECTOR_FIELD && source_within(&sim.field, slot)
}

pub(super) fn source_within(source: &str, name: &str) -> bool {
    source == name || source.split_once('.').is_some_and(|(root, _)| root == name)
}

/// The named vector read by this operand in place of the first SELECT's source.
fn positional_sources<'a>(
    query: &'a Query,
    columns: &'a SelectColumns,
) -> impl Iterator<Item = &'a str> {
    let first = super::super::set_operation::select_fields(&query.select.columns).0;
    let own = super::super::set_operation::select_fields(columns).0;
    let order = super::super::compound_window(query)
        .order_by
        .unwrap_or_default();
    first.iter().zip(own).filter_map(move |(first, own)| {
        let named = order.iter().any(|item| {
            matches!(&item.expr, OrderByExpr::Similarity(sim)
            if sim.field == first.name && sim.field != super::super::PRIMARY_VECTOR_FIELD)
        });
        (first.name != own.name && named).then_some(own.name.as_str())
    })
}

pub(in crate::velesql) fn orders_named_vector(query: &Query) -> bool {
    super::super::compound_window(query).order_by.unwrap_or_default().iter()
        .any(|item| matches!(&item.expr, OrderByExpr::Similarity(sim) if sim.field != super::super::PRIMARY_VECTOR_FIELD))
}

pub(super) fn has_positional_vector(query: &Query) -> bool {
    orders_named_vector(query)
        && query
            .compound
            .iter()
            .flat_map(|c| &c.operations)
            .any(|(_, operand)| positional_sources(query, &operand.columns).next().is_some())
}

pub(super) fn retains_vector_input(
    query: &Query,
    columns: &SelectColumns,
    order: &[SelectOrderBy],
    name: &str,
) -> bool {
    order
        .iter()
        .any(|item| reads_vector_input(&item.expr, name))
        || (orders_named_vector(query)
            && positional_sources(query, columns).any(|source| source_within(source, name)))
}
