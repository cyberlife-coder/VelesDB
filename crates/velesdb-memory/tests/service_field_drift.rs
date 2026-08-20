//! `MemoryService` exists twice by necessity — its default type parameter
//! names `NativeStore`, a type that does not exist without `persistence` —
//! but the *shape* of the service is one contract: what varies per feature
//! is the type of `GenerationGate`, never the field list (#2017). This test
//! reads the source and fails any change that lets the two definitions
//! diverge again, which is exactly the failure mode the issue records: a
//! field added under one cfg silently missing under the other.

/// Extract the field names of every `struct MemoryService` definition in
/// `service.rs`, in source order.
fn memory_service_field_lists() -> Vec<Vec<String>> {
    let source = include_str!("../src/service.rs");
    let mut lists = Vec::new();
    let mut rest = source;
    while let Some(pos) = rest.find("pub struct MemoryService<") {
        let body_start = match rest[pos..].find('{') {
            Some(brace) => pos + brace + 1,
            None => break,
        };
        let body_end = rest[body_start..]
            .find('}')
            .map_or(rest.len(), |end| body_start + end);
        let fields: Vec<String> = rest[body_start..body_end]
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                let (name, _ty) = line.split_once(':')?;
                Some(name.trim().to_string())
            })
            .collect();
        lists.push(fields);
        rest = &rest[body_end..];
    }
    lists
}

#[test]
fn both_memory_service_definitions_declare_the_same_fields() {
    let lists = memory_service_field_lists();
    assert_eq!(
        lists.len(),
        2,
        "expected exactly two cfg-gated MemoryService definitions, found {}: \
         if the dual definition was unified, delete this test; if a third \
         appeared, it needs the same field contract",
        lists.len()
    );
    assert_eq!(
        lists[0], lists[1],
        "the two MemoryService definitions declare different fields — a field \
         added under one cfg is silently absent under the other (#2017); add \
         it to both, cfg-typing the field's *type* if it must vary"
    );
    assert!(
        lists[0].contains(&"generation_gate".to_string()),
        "generation_gate left the field contract — if that is deliberate, \
         update this test and the GenerationGate doc together"
    );
}
