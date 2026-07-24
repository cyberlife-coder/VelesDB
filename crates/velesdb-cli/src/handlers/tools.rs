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
            println!("\n{}", "SIMD Native Configuration".bold().underline());
            println!("  Using simd_native with tiered dispatch:");
            println!("  - AVX-512: 4/2/1 accumulators based on vector size");
            println!("  - AVX2: 4-acc (>1024D), 2-acc (64-1023D), 1-acc (<64D)");
            println!("  - ARM NEON: 128-bit SIMD");
            println!("  - Scalar: fallback for small vectors");
            println!("\n{}", "Available Functions:".cyan());
            println!("  - dot_product_native()");
            println!("  - cosine_similarity_native()");
            println!("  - euclidean_native()");
            println!("  - hamming_distance_native()");
            println!("  - jaccard_similarity_native()");
            println!("  - batch_dot_product_native() (with prefetching)");
            println!();
        }
        SimdAction::Benchmark => {
            println!("{}", "SIMD micro-benchmarks removed.".yellow());
            println!("Use 'cargo bench --bench simd_benchmark' for detailed benchmarks.");
            println!();
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

    if format == "json" {
        let json = plan
            .to_json()
            .map_err(|e| anyhow::anyhow!("JSON serialization error: {e}"))?;
        println!("{json}");
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
                println!(
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
    println!("{}", "\u{274c} No license found".red().bold());
    println!("\n{}", "To activate a license:".cyan());
    println!("  velesdb license activate <license_key>");
    println!("\n{}", "License keys are stored in:".cyan());
    if let Ok(path) = license::get_license_config_path() {
        println!("  {}", path.display());
    }
}

/// Activates a license key after validation.
fn handle_license_activate(key: &str) -> Result<()> {
    let public_key = get_public_key_or_fallback();
    match license::validate_license(key, &public_key) {
        Ok(info) => {
            license::save_license_key(key)?;
            println!(
                "{}",
                "\u{2705} License activated successfully!".green().bold()
            );
            println!();
            license::display_license_info(&info);
            if let Ok(path) = license::get_license_config_path() {
                println!("{}", "License saved to:".cyan());
                println!("  {}", path.display());
            }
        }
        Err(e) => {
            println!(
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
    println!("\n{}", "Please check:".yellow());
    println!("  \u{2022} License key format is correct (base64_payload.base64_signature)");
    println!("  \u{2022} License has not expired");
    println!("  \u{2022} Public key is correctly set in VELESDB_LICENSE_PUBLIC_KEY");
}

/// Verifies a license key against a provided public key.
fn handle_license_verify(key: &str, public_key: &str) -> Result<()> {
    match license::validate_license(key, public_key) {
        Ok(info) => {
            println!("{}", "\u{2705} License is VALID".green().bold());
            println!();
            license::display_license_info(&info);
        }
        Err(e) => {
            println!(
                "{} {}",
                "\u{274c} License verification failed:".red().bold(),
                e
            );
            println!("\n{}", "Possible reasons:".yellow());
            println!("  \u{2022} Invalid signature (license may have been tampered with)");
            println!("  \u{2022} Wrong public key");
            println!("  \u{2022} License has expired");
            println!("  \u{2022} Malformed license format");
            std::process::exit(1);
        }
    }
    Ok(())
}

/// Reads the public key from environment, falling back to a dev key with a warning.
fn get_public_key_or_fallback() -> String {
    std::env::var("VELESDB_LICENSE_PUBLIC_KEY").unwrap_or_else(|_| {
        println!(
            "{}",
            "\u{26a0}\u{fe0f}  Warning: VELESDB_LICENSE_PUBLIC_KEY not set in environment".yellow()
        );
        println!("   Set it with: export VELESDB_LICENSE_PUBLIC_KEY=<base64_key>");
        println!("   Using embedded development key for validation...\n");
        "MCowBQYDK2VwAyEADevelopmentKeyReplaceMeInProd==".to_string()
    })
}
