//! LLM-free measurement of the graph's *explanation* value.
//!
//! LoCoMo's answer-accuracy cannot isolate the multi-hop graph's real strength:
//! connecting evidence that lies scattered across sessions. Here we measure it
//! directly. For every question with ≥2 gold `evidence` `dia_id`s we ask: of
//! those evidence facts, what fraction does plain top-`k` **vector** recall
//! surface, versus vector **plus** the facts `why()` reaches by graph traversal?
//! No generator, no judge — just retrieval, so it runs over all conversations in
//! seconds and the number is fully reproducible.

use std::collections::HashSet;
use std::error::Error;

use crate::dataset::{Category, Qa};
use crate::eval::EvalCfg;
use crate::ingest::Store;

/// Coverage tallies for one category.
#[derive(Clone, Copy, Default)]
struct Coverage {
    /// Multi-evidence questions seen.
    n: u32,
    /// Sum of per-question vector-only evidence coverage (0..=1).
    vector_sum: f64,
    /// Sum of per-question vector+graph evidence coverage (0..=1).
    fused_sum: f64,
    /// Questions where the graph covered evidence the vector missed.
    graph_helped: u32,
}

impl Coverage {
    fn add(&mut self, vector: f64, fused: f64) {
        self.n += 1;
        self.vector_sum += vector;
        self.fused_sum += fused;
        if fused > vector + f64::EPSILON {
            self.graph_helped += 1;
        }
    }

    fn merge(self, other: Self) -> Self {
        Self {
            n: self.n + other.n,
            vector_sum: self.vector_sum + other.vector_sum,
            fused_sum: self.fused_sum + other.fused_sum,
            graph_helped: self.graph_helped + other.graph_helped,
        }
    }
}

/// Per-category evidence-coverage report for the explanation benchmark.
#[derive(Default)]
pub struct ExplainReport {
    cells: [Coverage; 5],
}

impl ExplainReport {
    /// Measure one question and fold it in (multi-evidence questions only).
    pub fn record(&mut self, store: &Store, qa: &Qa, cfg: EvalCfg) -> Result<(), Box<dyn Error>> {
        if qa.evidence.len() < 2 {
            return Ok(());
        }
        let evidence: HashSet<&str> = qa.evidence.iter().map(String::as_str).collect();
        let vector = coverage(&vector_dia_ids(store, qa, cfg.k)?, &evidence);
        let mut fused_ids = vector_dia_ids(store, qa, cfg.k)?;
        fused_ids.extend(graph_dia_ids(store, qa, cfg.hops)?);
        let fused = coverage(&fused_ids, &evidence);
        self.cells[qa.category.index()].add(vector, fused);
        Ok(())
    }

    /// Print the coverage table and the headline graph lift.
    pub fn print(&self, cfg: EvalCfg, samples: usize) {
        println!(
            "\nVelesDB-memory — LoCoMo explanation benchmark (graph connects scattered evidence)"
        );
        println!(
            "embedder: ollama / all-minilm   ·   {samples} conversation(s)   ·   \
multi-evidence questions only, k={}, {} hops\n",
            cfg.k, cfg.hops
        );
        println!(
            "  {:<13}{:>5}   {:>10} {:>10} {:>9}",
            "category", "n", "vector-cov", "fused-cov", "graph+"
        );
        for category in Category::ALL {
            self.print_row(category.label(), self.cells[category.index()]);
        }
        let total = self
            .cells
            .iter()
            .fold(Coverage::default(), |a, c| a.merge(*c));
        self.print_row("ALL", total);
        println!(
            "\n→ evidence coverage: vector {:.0}% → +graph {:.0}% ({:+.0} pp)   ·   \
graph completed missing evidence on {}/{} questions",
            pct(total.vector_sum, total.n),
            pct(total.fused_sum, total.n),
            pct(total.fused_sum, total.n) - pct(total.vector_sum, total.n),
            total.graph_helped,
            total.n,
        );
        println!("  (coverage = share of a question's gold evidence dia_ids present in the retrieved facts)");
    }

    fn print_row(&self, label: &str, cell: Coverage) {
        println!(
            "  {:<13}{:>5}   {:>9.0}% {:>9.0}% {:>9}",
            label,
            cell.n,
            pct(cell.vector_sum, cell.n),
            pct(cell.fused_sum, cell.n),
            cell.graph_helped,
        );
    }
}

/// The `dia_id`s covered by the top-`k` vector facts for `qa`.
fn vector_dia_ids(store: &Store, qa: &Qa, k: usize) -> Result<HashSet<String>, Box<dyn Error>> {
    let hits = store
        .svc
        .recall(&qa.question, k.saturating_mul(3).saturating_add(8), None)?;
    let mut ids = HashSet::new();
    let mut facts = 0;
    for hit in hits {
        if !store.is_fact(hit.id) {
            continue;
        }
        ids.extend(store.dia_ids(hit.id).iter().cloned());
        facts += 1;
        if facts == k {
            break;
        }
    }
    Ok(ids)
}

/// The `dia_id`s covered by facts `why()` reaches by traversal (hop ≥ 1).
fn graph_dia_ids(store: &Store, qa: &Qa, hops: usize) -> Result<HashSet<String>, Box<dyn Error>> {
    let explanation = store.svc.why(&qa.question, hops, None)?;
    let mut ids = HashSet::new();
    for node in explanation.nodes {
        if node.hop >= 1 && store.is_fact(node.id) {
            ids.extend(store.dia_ids(node.id).iter().cloned());
        }
    }
    Ok(ids)
}

/// Fraction of `evidence` present in `covered` (0..=1).
fn coverage(covered: &HashSet<String>, evidence: &HashSet<&str>) -> f64 {
    let hit = evidence.iter().filter(|id| covered.contains(**id)).count();
    ratio(hit, evidence.len())
}

/// `num / den` as a fraction, guarding a zero denominator.
fn ratio(num: usize, den: usize) -> f64 {
    if den == 0 {
        return 0.0;
    }
    let num = u32::try_from(num).unwrap_or(u32::MAX);
    let den = u32::try_from(den).unwrap_or(u32::MAX);
    f64::from(num) / f64::from(den)
}

/// Mean of a coverage sum as a percentage.
fn pct(sum: f64, n: u32) -> f64 {
    if n == 0 {
        return 0.0;
    }
    100.0 * sum / f64::from(n)
}
