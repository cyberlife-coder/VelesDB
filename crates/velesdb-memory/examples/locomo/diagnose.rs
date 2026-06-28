//! Root-cause decomposition of the evidence-coverage gap, LLM-free.
//!
//! Budgeted evidence-recall sits below 100% — but is the gold evidence *missing
//! from the store* (an extraction failure: the extractor never produced a fact
//! for that turn) or *present but not retrieved* (a retrieval/graph failure)?
//! This pass splits the gap into those two, per category, so we know which lever
//! to pull. For each answerable, evidence-bearing question we measure, over its
//! gold evidence `dia_id`s:
//! - **extracted** = fraction that a fact exists for at all (the ceiling),
//! - **retrieved** = fraction present in the budgeted top-`k`.
//!
//! `extracted − retrieved` is the retrieval loss; `1 − extracted` is the
//! extraction loss.

use std::collections::HashSet;
use std::error::Error;

use crate::dataset::{Category, Qa};
use crate::eval::{self, EvalCfg};
use crate::ingest::Store;

#[derive(Clone, Copy, Default)]
struct Cell {
    n: u32,
    extracted_sum: f64,
    retrieved_sum: f64,
}

impl Cell {
    fn add(&mut self, extracted: f64, retrieved: f64) {
        self.n += 1;
        self.extracted_sum += extracted;
        self.retrieved_sum += retrieved;
    }

    fn merge(self, other: Self) -> Self {
        Self {
            n: self.n + other.n,
            extracted_sum: self.extracted_sum + other.extracted_sum,
            retrieved_sum: self.retrieved_sum + other.retrieved_sum,
        }
    }
}

/// Per-category extraction-vs-retrieval coverage report.
#[derive(Default)]
pub struct DiagnoseReport {
    cells: [Cell; Category::COUNT],
}

impl DiagnoseReport {
    /// Measure one question and fold it in (answerable, evidence-bearing only).
    pub fn record(
        &mut self,
        store: &Store,
        extracted_ids: &HashSet<&str>,
        qa: &Qa,
        cfg: EvalCfg,
    ) -> Result<(), Box<dyn Error>> {
        if qa.category.is_adversarial() || qa.evidence.is_empty() {
            return Ok(());
        }
        let gold: HashSet<&str> = qa.evidence.iter().map(String::as_str).collect();
        let extracted = coverage(&gold, |id| extracted_ids.contains(id));

        // Retrieval coverage uses the real budgeted fused retrieval (graph on).
        let retrieved_sets = eval::retrieved_dia_ids(store, &qa.question, cfg, true, qa.category)?;
        let retrieved_ids: HashSet<&str> = retrieved_sets
            .iter()
            .flat_map(|ids| ids.iter().map(String::as_str))
            .collect();
        let retrieved = coverage(&gold, |id| retrieved_ids.contains(id));

        self.cells[qa.category.index()].add(extracted, retrieved);
        Ok(())
    }

    /// Print the decomposition table and the headline split.
    pub fn print(&self, cfg: EvalCfg, samples: usize) {
        println!("\nVelesDB-memory — LoCoMo coverage diagnosis (extraction vs retrieval)");
        println!(
            "embedder: see run · {samples} conversation(s) · answerable evidence-bearing questions, k={}\n",
            cfg.k
        );
        println!(
            "  {:<13}{:>5}   {:>10} {:>10} {:>11} {:>11}",
            "category", "n", "extracted", "retrieved", "extr-loss", "retr-loss"
        );
        for category in Category::ALL {
            self.print_row(category.label(), self.cells[category.index()]);
        }
        let total = self.cells.iter().fold(Cell::default(), |a, c| a.merge(*c));
        self.print_row("ALL", total);
        println!(
            "\n→ of the gold evidence: {:.0}% is extractable, {:.0}% is retrieved.   \
extraction loss {:.0}pp · retrieval loss {:.0}pp",
            pct(total.extracted_sum, total.n),
            pct(total.retrieved_sum, total.n),
            100.0 - pct(total.extracted_sum, total.n),
            pct(total.extracted_sum, total.n) - pct(total.retrieved_sum, total.n),
        );
        println!(
            "  (extr-loss = gold turns with NO extracted fact; retr-loss = extracted but outside top-k)"
        );
    }

    #[allow(clippy::unused_self)]
    fn print_row(&self, label: &str, cell: Cell) {
        let extr = pct(cell.extracted_sum, cell.n);
        let retr = pct(cell.retrieved_sum, cell.n);
        println!(
            "  {:<13}{:>5}   {:>9.0}% {:>9.0}% {:>10.0} {:>10.0}",
            label,
            cell.n,
            extr,
            retr,
            100.0 - extr,
            extr - retr,
        );
    }
}

/// Fraction of `gold` ids for which `present` holds.
fn coverage(gold: &HashSet<&str>, present: impl Fn(&str) -> bool) -> f64 {
    let hit = gold.iter().filter(|id| present(id)).count();
    if gold.is_empty() {
        return 0.0;
    }
    let hit = u32::try_from(hit).unwrap_or(u32::MAX);
    let total = u32::try_from(gold.len()).unwrap_or(u32::MAX);
    f64::from(hit) / f64::from(total)
}

/// Mean of a coverage sum as a percentage.
fn pct(sum: f64, n: u32) -> f64 {
    if n == 0 {
        return 0.0;
    }
    100.0 * sum / f64::from(n)
}
