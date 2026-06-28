//! LLM-free measurement of *budgeted* retrieval quality — the honest fast proxy
//! for the QA-accuracy effect of fusion.
//!
//! The explanation benchmark ([`crate::explain`]) unions `vector ∪ graph`, so it
//! can only ever raise coverage and never shows the graph hurting. The actual
//! answer path does not: it runs the budgeted [`crate::eval::retrieve`], where a
//! graph-injected distractor that displaces a real evidence fact at the top-`k`
//! boundary is a net loss. This pass scores that exact budgeted fact set against
//! the gold `evidence`, so a config that does not lift evidence-recall under
//! budget cannot lift accuracy — letting us tune fusion in seconds (extraction
//! and embeddings are cached; no generator, no judge) before paying for a full
//! accuracy run.

use std::collections::HashSet;
use std::error::Error;

use crate::dataset::{Category, Qa};
use crate::eval::{self, EvalCfg};
use crate::ingest::Store;

/// Per-category budgeted-recall tallies for vector vs fused retrieval.
#[derive(Clone, Copy, Default)]
struct Cell {
    /// Answerable, evidence-bearing questions seen.
    n: u32,
    /// Sum of per-question vector-only evidence recall@k (0..=1).
    vector_sum: f64,
    /// Sum of per-question fused evidence recall@k (0..=1).
    fused_sum: f64,
    /// Questions where fusion *raised* evidence recall (the graph's upside).
    fused_better: u32,
    /// Questions where fusion *lowered* it — a distractor evicted evidence
    /// under the fixed budget. This is the cost the explanation metric hides.
    fused_worse: u32,
}

impl Cell {
    fn add(&mut self, vector: f64, fused: f64) {
        self.n += 1;
        self.vector_sum += vector;
        self.fused_sum += fused;
        if fused > vector + f64::EPSILON {
            self.fused_better += 1;
        } else if fused + f64::EPSILON < vector {
            self.fused_worse += 1;
        }
    }

    fn merge(self, other: Self) -> Self {
        Self {
            n: self.n + other.n,
            vector_sum: self.vector_sum + other.vector_sum,
            fused_sum: self.fused_sum + other.fused_sum,
            fused_better: self.fused_better + other.fused_better,
            fused_worse: self.fused_worse + other.fused_worse,
        }
    }
}

/// Per-category budgeted-recall report for the retrieval benchmark.
#[derive(Default)]
pub struct RetrievalReport {
    cells: [Cell; Category::COUNT],
}

impl RetrievalReport {
    /// Measure one question under both modes and fold it in. Adversarial and
    /// evidence-less questions are skipped: there is no gold evidence to recall.
    pub fn record(&mut self, store: &Store, qa: &Qa, cfg: EvalCfg) -> Result<(), Box<dyn Error>> {
        if qa.category.is_adversarial() || qa.evidence.is_empty() {
            return Ok(());
        }
        let gold: HashSet<&str> = qa.evidence.iter().map(String::as_str).collect();
        let cat = qa.category;
        let vector = recall(
            &eval::retrieved_dia_ids(store, &qa.question, cfg, false, cat)?,
            &gold,
        );
        let fused = recall(
            &eval::retrieved_dia_ids(store, &qa.question, cfg, true, cat)?,
            &gold,
        );
        self.cells[cat.index()].add(vector, fused);
        Ok(())
    }

    /// Print the per-category recall table and the headline fused-vs-vector delta.
    pub fn print(&self, cfg: EvalCfg, samples: usize) {
        println!("\nVelesDB-memory — LoCoMo retrieval benchmark (budgeted evidence-recall@k)");
        println!(
            "embedder: ollama / all-minilm   ·   {samples} conversation(s)   ·   \
answerable evidence-bearing questions, k={}, graph_boost={}, {} hops   ·   \
idf-weight={}, multihop-only={}, seed-breadth={}\n",
            cfg.k, cfg.graph_boost, cfg.hops, cfg.idf_weight, cfg.multihop_only, cfg.seed_breadth
        );
        println!(
            "  {:<13}{:>5}   {:>9} {:>9}   {:>7} {:>7}",
            "category", "n", "vec-rec", "fus-rec", "better", "worse"
        );
        for category in Category::ALL {
            self.print_row(category.label(), self.cells[category.index()]);
        }
        let total = self.cells.iter().fold(Cell::default(), |a, c| a.merge(*c));
        self.print_row("ALL", total);
        println!(
            "\n→ evidence recall@k: vector {:.1}% → fused {:.1}% ({:+.1} pp)   ·   \
fusion helped {} / hurt {} of {} questions",
            pct(total.vector_sum, total.n),
            pct(total.fused_sum, total.n),
            pct(total.fused_sum, total.n) - pct(total.vector_sum, total.n),
            total.fused_better,
            total.fused_worse,
            total.n,
        );
        println!(
            "  (recall@k = share of a question's gold evidence dia_ids present in the budgeted top-k facts)"
        );
    }

    #[allow(clippy::unused_self)]
    fn print_row(&self, label: &str, cell: Cell) {
        println!(
            "  {:<13}{:>5}   {:>8.1}% {:>8.1}%   {:>7} {:>7}",
            label,
            cell.n,
            pct(cell.vector_sum, cell.n),
            pct(cell.fused_sum, cell.n),
            cell.fused_better,
            cell.fused_worse,
        );
    }
}

/// Fraction of `gold` evidence `dia_ids` present across the retrieved facts' source
/// `dia_ids` (0..=1).
fn recall(retrieved: &[Vec<String>], gold: &HashSet<&str>) -> f64 {
    let covered: HashSet<&str> = retrieved
        .iter()
        .flat_map(|ids| ids.iter().map(String::as_str))
        .collect();
    let hit = gold.iter().filter(|id| covered.contains(**id)).count();
    ratio(hit, gold.len())
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

/// Mean of a recall sum as a percentage.
fn pct(sum: f64, n: u32) -> f64 {
    if n == 0 {
        return 0.0;
    }
    100.0 * sum / f64::from(n)
}
