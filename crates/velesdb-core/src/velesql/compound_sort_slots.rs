//! Temporary fields used only while ordering a compound. Their names are
//! chosen after inspecting the input documents and the projected columns.
use super::{
    occupy_order_names, qualified_wildcard, reads_vector_input, Column, CompoundProjection,
    FxHashSet, ProjectedField, Query, SearchResult, SelectOrderBy, Value,
};

/// Scalar values whose normal output slots shadow a vector input. The
/// alternative literal keys exist only while the compound is being ordered.
pub(in crate::velesql) struct VectorSortSlots(Vec<DeferredMember>);

struct DeferredMember {
    name: String,
    temporary: String,
    // These members can be absent when a different operand supplies the vector.
    original_at: Option<usize>,
}

impl VectorSortSlots {
    pub(in crate::velesql) fn publish(&self, page: &mut [SearchResult]) {
        for payload in page.iter_mut().filter_map(|row| row.point.payload.as_mut()) {
            for field in &self.0 {
                let Some(mut value) = exchange_member(payload, &field.temporary, None) else {
                    continue;
                };
                let restored = if field.original_at.is_some() {
                    value.as_array_mut().and_then(Vec::pop)
                } else {
                    Some(value)
                };
                exchange_member(payload, &field.name, restored);
            }
        }
    }

    pub(in crate::velesql) fn replaced_members(&self, columns: &[Column]) -> Vec<(&str, &str)> {
        self.0
            .iter()
            .filter_map(|field| {
                let col = columns.get(field.original_at?)?;
                (col.name != field.name).then_some((field.name.as_str(), field.temporary.as_str()))
            })
            .collect()
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
        for &(output, field) in self.named.iter().flatten() {
            occupied.extend([output, field.name()]);
        }
        for field in fields {
            occupied.insert(field.name());
        }
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
        Some(VectorSortSlots(fields))
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
                    .0
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
            .filter(|(name, i)| {
                query
                    .compound
                    .iter()
                    .flat_map(|compound| &compound.operations)
                    .any(|(_, select)| {
                        crate::velesql::set_operation::select_fields(&select.columns)
                            .0
                            .get(*i)
                            .is_some_and(|c| c.name != *name)
                    })
            })
            .collect()
    }
}
