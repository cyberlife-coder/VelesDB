//! Benchmarks for MATCH parsing.
//! Required by US-001 `DoD`: Parsing < 1µs for simple patterns.
//!
//! Each case parses a whole `MATCH` statement through [`Parser::parse`], the
//! only MATCH parser core has: pattern parsing is not reachable on its own.
//! The ids differ from the earlier `parse_node_*` / `parse_match_*` ones,
//! which timed a string-scanning pattern parser no query went through, so a
//! baseline recorded under those names is not comparable.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use velesdb_core::velesql::Parser;

fn bench_parse(c: &mut Criterion, name: &str, query: &'static str) {
    c.bench_function(name, |b| {
        b.iter(|| {
            let _ = Parser::parse(black_box(query));
        });
    });
}

fn bench_parse_simple_node(c: &mut Criterion) {
    bench_parse(c, "match_stmt_node", "MATCH (n) RETURN n");
}

fn bench_parse_node_with_label(c: &mut Criterion) {
    bench_parse(c, "match_stmt_node_with_label", "MATCH (n:Person) RETURN n");
}

fn bench_parse_node_with_props(c: &mut Criterion) {
    bench_parse(
        c,
        "match_stmt_node_with_props",
        "MATCH (n:Person {name: 'Alice', age: 30}) RETURN n",
    );
}

fn bench_parse_relationship(c: &mut Criterion) {
    bench_parse(
        c,
        "match_stmt_relationship",
        "MATCH (a)-[r:WROTE]->(b) RETURN a",
    );
}

fn bench_parse_match_simple(c: &mut Criterion) {
    bench_parse(
        c,
        "match_stmt_path",
        "MATCH (p:Person)-[:WROTE]->(a:Article) RETURN a.title",
    );
}

fn bench_parse_match_with_where(c: &mut Criterion) {
    bench_parse(
        c,
        "match_stmt_path_with_where",
        "MATCH (p:Person)-[:WROTE]->(a) WHERE p.age > 18 RETURN a",
    );
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
