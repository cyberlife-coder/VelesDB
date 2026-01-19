//! Benchmarks for MATCH clause parser.
//! Required by US-001 `DoD`: Parsing < 1µs for simple patterns.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use velesdb_core::velesql::match_clause::{
    parse_match_clause, parse_node_pattern, parse_relationship_pattern,
};

fn bench_parse_simple_node(c: &mut Criterion) {
    c.bench_function("parse_node_simple", |b| {
        b.iter(|| parse_node_pattern(black_box("(n)")));
    });
}

fn bench_parse_node_with_label(c: &mut Criterion) {
    c.bench_function("parse_node_with_label", |b| {
        b.iter(|| parse_node_pattern(black_box("(n:Person)")));
    });
}

fn bench_parse_node_with_props(c: &mut Criterion) {
    c.bench_function("parse_node_with_props", |b| {
        b.iter(|| parse_node_pattern(black_box("(n:Person {name: 'Alice', age: 30})")));
    });
}

fn bench_parse_relationship(c: &mut Criterion) {
    c.bench_function("parse_relationship", |b| {
        b.iter(|| parse_relationship_pattern(black_box("-[r:WROTE]->")));
    });
}

fn bench_parse_match_simple(c: &mut Criterion) {
    c.bench_function("parse_match_simple", |b| {
        b.iter(|| {
            let _ = parse_match_clause(black_box(
                "MATCH (p:Person)-[:WROTE]->(a:Article) RETURN a.title",
            ));
        });
    });
}

fn bench_parse_match_with_where(c: &mut Criterion) {
    c.bench_function("parse_match_with_where", |b| {
        b.iter(|| {
            let _ = parse_match_clause(black_box(
                "MATCH (p:Person)-[:WROTE]->(a) WHERE p.age > 18 RETURN a",
            ));
        });
    });
}

criterion_group!(
    match_parser_benches,
    bench_parse_simple_node,
    bench_parse_node_with_label,
    bench_parse_node_with_props,
    bench_parse_relationship,
    bench_parse_match_simple,
    bench_parse_match_with_where,
);
criterion_main!(match_parser_benches);
