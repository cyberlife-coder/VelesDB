//! Console UI formatting for the migration wizard.

use console::{style, Style, Term};

use crate::connectors::SourceSchema;
use crate::pipeline::MigrationStats;
use crate::redact::redact_url;

use super::WizardConfig;

/// Builds the "connecting" line with credentials stripped from the URL.
pub(crate) fn format_connecting(url: &str) -> String {
    format!(
        "{} Connecting to {}...",
        style("🔌").bold(),
        redact_url(url)
    )
}

/// Builds the post-migration success block (stats + quick-start hints).
pub(crate) fn format_success(stats: &MigrationStats, config: &WizardConfig) -> String {
    let green = Style::new().green().bold();
    let mut out = format!("\n{}\n\n", green.apply_to("✅ Migration Complete!"));
    out.push_str(&format_stats_lines(stats));
    out.push_str(&format_quickstart_lines(config));
    out
}

/// Builds the migrated-vectors / graph-edges stat lines.
fn format_stats_lines(stats: &MigrationStats) -> String {
    use std::fmt::Write as _;
    let bold = Style::new().bold();
    let mut out = format!(
        "   {} {}\n   {} {:.1}s\n   {} {:.0} vec/s\n",
        bold.apply_to("Vectors migrated:"),
        stats.loaded,
        bold.apply_to("Duration:        "),
        stats.duration_secs,
        bold.apply_to("Throughput:      "),
        stats.throughput()
    );
    if stats.failed > 0 {
        let _ = writeln!(
            out,
            "   {} {} (skipped)",
            style("Failed:").yellow(),
            stats.failed
        );
    }
    if stats.relations_processed > 0 {
        out.push_str(&format_relations_lines(stats));
    }
    out
}

/// Builds the graph-migration stat lines (only when relations were processed).
fn format_relations_lines(stats: &MigrationStats) -> String {
    use std::fmt::Write as _;
    let bold = Style::new().bold();
    let mut out = format!(
        "\n   {} {}\n   {} {}\n",
        bold.apply_to("Graph edges created:"),
        stats.edges_created,
        bold.apply_to("Relations processed: "),
        stats.relations_processed
    );
    if stats.edges_failed > 0 {
        let _ = writeln!(
            out,
            "   {} {} (skipped)",
            style("Edges failed:").yellow(),
            stats.edges_failed
        );
    }
    out
}

/// Builds the "Quick start" command hints.
fn format_quickstart_lines(config: &WizardConfig) -> String {
    let bold = Style::new().bold();
    let dim = Style::new().dim();
    format!(
        "\n{}\n   {} velesdb serve --data {}\n   {} velesdb query \"SELECT * FROM {} ORDER BY vector <-> [...] LIMIT 10\"\n",
        bold.apply_to("💡 Quick start:"),
        dim.apply_to("$"),
        config.dest_path,
        dim.apply_to("$"),
        config.collection
    )
}

/// Console UI handler for the wizard.
pub struct WizardUI {
    #[allow(dead_code)]
    term: Term,
}

impl Default for WizardUI {
    fn default() -> Self {
        Self::new()
    }
}

impl WizardUI {
    /// Creates a new UI handler.
    pub fn new() -> Self {
        Self {
            term: Term::stdout(),
        }
    }

    /// Prints the wizard header.
    pub fn print_header(&self) {
        let cyan = Style::new().cyan().bold();

        println!();
        println!(
            "{}",
            cyan.apply_to("╔═══════════════════════════════════════════════════════════════╗")
        );
        println!(
            "{}",
            cyan.apply_to("║         🚀 VELESDB MIGRATION WIZARD                           ║")
        );
        println!(
            "{}",
            cyan.apply_to("║         Migrate your vectors in under 60 seconds              ║")
        );
        println!(
            "{}",
            cyan.apply_to("╚═══════════════════════════════════════════════════════════════╝")
        );
        println!();
    }

    /// Prints connecting message.
    pub fn print_connecting(&self, url: &str) {
        println!();
        println!("{}", format_connecting(url));
    }

    /// Prints schema discovery results.
    pub fn print_schema_discovered(&self, schema: &SourceSchema) {
        let green = Style::new().green().bold();
        let bold = Style::new().bold();

        println!();
        println!("{} Schema Detected!", green.apply_to("✅"));
        println!("┌─────────────────────────────────────────────────────────────┐");
        println!("│ {} {}", bold.apply_to("Source Type:"), schema.source_type);
        println!("│ {} {}", bold.apply_to("Collection: "), schema.collection);
        println!(
            "│ {} {}",
            bold.apply_to("Dimension:  "),
            if schema.dimension > 0 {
                schema.dimension.to_string()
            } else {
                "auto-detect".to_string()
            }
        );
        println!(
            "│ {} {}",
            bold.apply_to("Count:      "),
            schema
                .total_count
                .map_or("unknown".to_string(), |c| format!("{c} vectors"))
        );
        println!("├─────────────────────────────────────────────────────────────┤");

        if schema.fields.is_empty() {
            println!(
                "│ {} (all fields will be migrated)",
                style("Metadata:").dim()
            );
        } else {
            println!("│ {}:", bold.apply_to("Metadata Fields"));
            for field in schema.fields.iter().take(10) {
                let indexed = if field.indexed { " [indexed]" } else { "" };
                println!("│   • {} ({}){}", field.name, field.field_type, indexed);
            }
            if schema.fields.len() > 10 {
                println!("│   ... and {} more", schema.fields.len() - 10);
            }
        }
        println!("└─────────────────────────────────────────────────────────────┘");
    }

    /// Prints migration starting message.
    pub fn print_starting_migration(&self) {
        println!();
        println!("{} Starting migration...", style("⚡").bold());
        println!();
    }

    /// Prints success message with stats.
    pub fn print_success(&self, stats: &MigrationStats, config: &WizardConfig) {
        println!("{}", format_success(stats, config));
    }

    /// Prints cancellation message.
    pub fn print_cancelled(&self) {
        println!();
        println!("{} Migration cancelled.", style("ℹ").blue());
    }

    /// Prints error message.
    pub fn print_error(&self, message: &str) {
        println!();
        println!("{} {}", style("❌").red().bold(), message);
    }
}
