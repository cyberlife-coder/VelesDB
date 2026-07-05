//! Result aggregation and the final report.
//!
//! Tallies accuracy (LLM judge), evidence-recall (deterministic), and mean F1
//! per category, for the vector-only and vector+graph modes side by side, then
//! prints the headline graph contribution.

use crate::dataset::Category;
use crate::eval::{EvalCfg, ModeResult};

/// Running totals for one (mode, category) cell.
#[derive(Clone, Copy, Default)]
struct Cell {
    n: u32,
    evidence: u32,
    correct: u32,
    f1_sum: f64,
    f1_n: u32,
}

impl Cell {
    fn add(&mut self, result: &ModeResult) {
        self.n += 1;
        self.evidence += u32::from(result.evidence_hit);
        self.correct += u32::from(result.correct);
        if let Some(f1) = result.f1 {
            self.f1_sum += f1;
            self.f1_n += 1;
        }
    }

    fn merge(&self, other: &Cell) -> Cell {
        Cell {
            n: self.n + other.n,
            evidence: self.evidence + other.evidence,
            correct: self.correct + other.correct,
            f1_sum: self.f1_sum + other.f1_sum,
            f1_n: self.f1_n + other.f1_n,
        }
    }

    fn accuracy(self) -> f64 {
        pct(self.correct, self.n)
    }

    fn evidence_overlap(self) -> f64 {
        pct(self.evidence, self.n)
    }

    fn mean_f1(self) -> f64 {
        if self.f1_n == 0 {
            return f64::NAN;
        }
        self.f1_sum / f64::from(self.f1_n)
    }
}

/// Per-category tallies for both retrieval modes.
#[derive(Default)]
pub struct Report {
    off: [Cell; 5],
    on: [Cell; 5],
}

impl Report {
    /// Record one QA outcome under one mode.
    pub fn record(&mut self, category: Category, graph_on: bool, result: &ModeResult) {
        let cells = if graph_on {
            &mut self.on
        } else {
            &mut self.off
        };
        cells[category.index()].add(result);
    }

    /// Print the full comparison table and the headline delta.
    pub fn print(
        &self,
        cfg: EvalCfg,
        model: &str,
        embed_model: &str,
        samples: usize,
        facts: usize,
    ) {
        println!("\nVelesDB-memory — LoCoMo benchmark (graph contribution)");
        println!(
            "judge/extractor: {model}   ·   embedder: ollama / {embed_model}   ·   \
{samples} conversation(s), {facts} extracted facts"
        );
        println!(
            "retrieval budget k={}  (graph mode: fused vector+graph rerank, boost {:.2}, {} hops)\n",
            cfg.k, cfg.graph_boost, cfg.hops
        );
        println!(
            "  {:<13}{:>5}   {:^21}   {:^21}",
            "", "n", "vector-only", "vector + graph"
        );
        println!(
            "  {:<13}{:>5}   {:>6} {:>6} {:>6}   {:>6} {:>6} {:>6}",
            "category", "", "acc", "ev", "f1", "acc", "ev", "f1"
        );
        for category in Category::ALL {
            print_row(
                category.label(),
                self.off[category.index()],
                self.on[category.index()],
            );
        }
        let off_total = answerable(&self.off);
        let on_total = answerable(&self.on);
        print_row("answerable", off_total, on_total);
        print_headline(off_total, on_total);
    }
}

/// One category (or total) row: accuracy / evidence-overlap / F1 per mode.
fn print_row(label: &str, off: Cell, on: Cell) {
    println!(
        "  {:<13}{:>5}   {:>5.0}% {:>5.0}% {:>6}   {:>5.0}% {:>5.0}% {:>6}",
        label,
        off.n,
        off.accuracy(),
        off.evidence_overlap(),
        fmt_f1(off.mean_f1()),
        on.accuracy(),
        on.evidence_overlap(),
        fmt_f1(on.mean_f1()),
    );
}

/// The bottom-line fusion (graph + `ColumnStore`) contribution, over answerable
/// items only — adversarial items are scored by abstention, so folding them in
/// would mix an abstention rate into the accuracy and obscure the effect.
fn print_headline(off: Cell, on: Cell) {
    println!(
        "\n→ fusion contribution (answerable only): accuracy {:.0}% → {:.0}% ({:+.0} pp)   ·   \
evidence-overlap {:.0}% → {:.0}% ({:+.0} pp)",
        off.accuracy(),
        on.accuracy(),
        on.accuracy() - off.accuracy(),
        off.evidence_overlap(),
        on.evidence_overlap(),
        on.evidence_overlap() - off.evidence_overlap(),
    );
    println!(
        "  (acc = LLM-judge accuracy; ev = evidence-overlap, ≥1 retrieved fact whose \
source ∈ gold evidence; f1 = mean token-F1.\n   adversarial acc = abstention rate, \
shown per-row but excluded from the answerable total and headline.)"
    );
}

/// Sum the answerable categories (everything but adversarial) into one total.
fn answerable(cells: &[Cell; 5]) -> Cell {
    Category::ALL
        .iter()
        .filter(|c| !c.is_adversarial())
        .fold(Cell::default(), |acc, c| acc.merge(&cells[c.index()]))
}

/// Percentage with a guarded zero denominator.
pub(crate) fn pct(num: u32, den: u32) -> f64 {
    if den == 0 {
        return 0.0;
    }
    100.0 * f64::from(num) / f64::from(den)
}

/// Render a mean-F1 cell, blanking the not-applicable adversarial case.
fn fmt_f1(value: f64) -> String {
    if value.is_nan() {
        return "  -  ".to_string();
    }
    format!("{value:.2}")
}
