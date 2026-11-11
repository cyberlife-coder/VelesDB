//! Handlers for utility/tooling commands: `simd`, `explain`, `completions`, `license`.

use std::path::Path;

use anyhow::Result;
use colored::Colorize;

use crate::commands::{LicenseAction, SimdAction};
use crate::license;

/// Handles the `simd` subcommand: SIMD diagnostics.
pub fn handle_simd(action: SimdAction) {
    match action {
        SimdAction::Info => {
            // What the dispatcher detected on this machine, read from core:
            // a fixed summary here described the design, not the machine, and
            // drifted from the thresholds the code applies (#1965).
            crate::info!("\n{}", "SIMD".bold().underline());
            crate::info!(
                "  Detected level: {}",
                velesdb_core::simd_native::simd_level()
            );
            crate::info!(
                "  Distance kernels (dot product, cosine, Euclidean, Hamming, Jaccard,\n  \
                 and their batch forms) dispatch to this level at runtime."
            );
            crate::info!();
        }
        SimdAction::Benchmark => {
            crate::info!("{}", "The CLI does not benchmark SIMD.".yellow());
            crate::info!("Run: cargo bench -p velesdb-core --bench simd_benchmark");
            crate::info!();
        }
    }
}

/// Handles the `explain` subcommand: query execution plan.
pub fn handle_explain(path: &Path, query: &str, format: &str) -> Result<()> {
    let db = crate::helpers::open_database(path)?;
    let parsed = velesdb_core::velesql::Parser::parse(query)
        .map_err(|e| anyhow::anyhow!("Parse error: {e}"))?;

    let plan = db
        .explain_query(&parsed)
        .map_err(|e| anyhow::anyhow!("Explain error: {e}"))?;

    if crate::output::is_machine(format) {
        let json = plan
            .to_json()
            .map_err(|e| anyhow::anyhow!("JSON serialization error: {e}"))?;
        let value: serde_json::Value =
            serde_json::from_str(&json).map_err(|e| anyhow::anyhow!("JSON parse error: {e}"))?;
        crate::output::print_machine(&value, format)?;
    } else {
        print_explain_tree(&plan);
    }
    Ok(())
}

/// Prints a query plan as a colored tree.
fn print_explain_tree(plan: &velesdb_core::velesql::QueryPlan) {
    println!("\n{}", "Query Execution Plan".bold().underline());
    println!("{}", plan.to_tree());
    println!(
        "  {} {:.3} ms",
        "Estimated cost:".cyan(),
        plan.estimated_cost_ms
    );
    if let Some(idx) = &plan.index_used {
        println!("  {} {:?}", "Index used:".cyan(), idx);
    }
    println!("  {} {:?}", "Filter strategy:".cyan(), plan.filter_strategy);
    if let Some(hit) = plan.cache_hit {
        println!(
            "  {} {}",
            "Cache hit:".cyan(),
            if hit { "yes".green() } else { "no".yellow() }
        );
    }
    if let Some(reuse) = plan.plan_reuse_count {
        println!("  {} {}", "Plan reuse count:".cyan(), reuse);
    }
    println!();
}

/// Handles the `completions` subcommand: generates shell completions.
pub fn handle_completions<C: clap::CommandFactory>(shell: clap_complete::Shell) {
    let mut cmd = C::command();
    clap_complete::generate(shell, &mut cmd, "velesdb", &mut std::io::stdout());
}

/// Handles the `license` subcommand: show, activate, verify.
pub fn handle_license(action: LicenseAction) -> Result<()> {
    match action {
        LicenseAction::Show => handle_license_show(),
        LicenseAction::Activate { key } => handle_license_activate(&key),
        LicenseAction::Verify { key, public_key } => handle_license_verify(&key, &public_key),
    }
}

/// Shows the current license status.
fn handle_license_show() -> Result<()> {
    if let Ok(key) = license::load_license_key() {
        let public_key = get_public_key_or_fallback();
        match license::validate_license(&key, &public_key) {
            Ok(info) => {
                license::display_license_info(&info);
            }
            Err(e) => {
                crate::info!(
                    "{} {}",
                    "\u{274c} License validation failed:".red().bold(),
                    e
                );
                std::process::exit(1);
            }
        }
    } else {
        print_no_license_found();
        std::process::exit(1);
    }
    Ok(())
}

/// Prints the "no license found" message with activation instructions.
fn print_no_license_found() {
    crate::info!("{}", "\u{274c} No license found".red().bold());
    crate::info!("\n{}", "To activate a license:".cyan());
    crate::info!("  velesdb license activate <license_key>");
    crate::info!("\n{}", "License keys are stored in:".cyan());
    if let Ok(path) = license::get_license_config_path() {
        crate::info!("  {}", path.display());
    }
}

/// Activates a license key after validation.
fn handle_license_activate(key: &str) -> Result<()> {
    let public_key = get_public_key_or_fallback();
    match license::validate_license(key, &public_key) {
        Ok(info) => {
            license::save_license_key(key)?;
            crate::info!(
                "{}",
                "\u{2705} License activated successfully!".green().bold()
            );
            crate::info!();
            license::display_license_info(&info);
            if let Ok(path) = license::get_license_config_path() {
                crate::info!("{}", "License saved to:".cyan());
                crate::info!("  {}", path.display());
            }
        }
        Err(e) => {
            crate::info!(
                "{} {}",
                "\u{274c} License activation failed:".red().bold(),
                e
            );
            print_license_troubleshooting();
            std::process::exit(1);
        }
    }
    Ok(())
}

/// Prints troubleshooting tips for license activation failure.
fn print_license_troubleshooting() {
    crate::info!("\n{}", "Please check:".yellow());
    crate::info!("  \u{2022} License key format is correct (base64_payload.base64_signature)");
    crate::info!("  \u{2022} License has not expired");
    crate::info!("  \u{2022} Public key is correctly set in VELESDB_LICENSE_PUBLIC_KEY");
}

/// Verifies a license key against a provided public key.
fn handle_license_verify(key: &str, public_key: &str) -> Result<()> {
    match license::validate_license(key, public_key) {
        Ok(info) => {
            crate::info!("{}", "\u{2705} License is VALID".green().bold());
            crate::info!();
            license::display_license_info(&info);
        }
        Err(e) => {
            crate::info!(
                "{} {}",
                "\u{274c} License verification failed:".red().bold(),
                e
            );
            crate::info!("\n{}", "Possible reasons:".yellow());
            crate::info!("  \u{2022} Invalid signature (license may have been tampered with)");
            crate::info!("  \u{2022} Wrong public key");
            crate::info!("  \u{2022} License has expired");
            crate::info!("  \u{2022} Malformed license format");
            std::process::exit(1);
        }
    }
    Ok(())
}

/// Reads the public key from environment, falling back to a dev key with a warning.
fn get_public_key_or_fallback() -> String {
    std::env::var("VELESDB_LICENSE_PUBLIC_KEY").unwrap_or_else(|_| {
        crate::info!(
            "{}",
            "\u{26a0}\u{fe0f}  Warning: VELESDB_LICENSE_PUBLIC_KEY not set in environment".yellow()
        );
        crate::info!("   Set it with: export VELESDB_LICENSE_PUBLIC_KEY=<base64_key>");
        crate::info!("   Using embedded development key for validation...\n");
        "MCowBQYDK2VwAyEADevelopmentKeyReplaceMeInProd==".to_string()
    })
}
