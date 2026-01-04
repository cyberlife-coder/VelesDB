//! Console UI formatting for the migration wizard.

use console::{style, Style, Term};

use crate::connectors::SourceSchema;
use crate::pipeline::MigrationStats;

use super::WizardConfig;

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
        println!("{} Connecting to {}...", style("🔌").bold(), url);
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
        let green = Style::new().green().bold();
        let bold = Style::new().bold();
        let dim = Style::new().dim();

        println!();
        println!("{}", green.apply_to("✅ Migration Complete!"));
        println!();
        println!("   {} {}", bold.apply_to("Vectors migrated:"), stats.loaded);
        println!(
            "   {} {:.1}s",
            bold.apply_to("Duration:        "),
            stats.duration_secs
        );
        println!(
            "   {} {:.0} vec/s",
            bold.apply_to("Throughput:      "),
            stats.throughput()
        );

        if stats.failed > 0 {
            println!(
                "   {} {} (skipped)",
                style("Failed:").yellow(),
                stats.failed
            );
        }

        println!();
        println!("{}", bold.apply_to("💡 Quick start:"));
        println!(
            "   {} velesdb serve --data {}",
            dim.apply_to("$"),
            config.dest_path
        );
        println!(
            "   {} velesdb query \"SELECT * FROM {} ORDER BY vector <-> [...] LIMIT 10\"",
            dim.apply_to("$"),
            config.collection
        );
        println!();
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
