//! Tests that the global output flags parse on every subcommand and that an
//! invalid `--output` value is a usage error (clap exits 2).

use super::*;
use clap::Parser;

#[test]
fn test_global_output_flags_parse() {
    let cli = Cli::try_parse_from([
        "velesdb",
        "--output",
        "jsonl",
        "--quiet",
        "--no-color",
        "info",
        "/tmp/db",
    ])
    .expect("global flags must parse before the subcommand");
    assert_eq!(cli.output, Some(output::GlobalFormat::Jsonl));
    assert!(cli.quiet);
    assert!(cli.no_color);
}

#[test]
fn test_global_flags_parse_after_subcommand() {
    // `global = true` lets the flags appear after the subcommand too.
    let cli = Cli::try_parse_from([
        "velesdb",
        "collection",
        "list",
        "/tmp/db",
        "--output",
        "csv",
    ])
    .expect("global flags must parse after the subcommand");
    assert_eq!(cli.output, Some(output::GlobalFormat::Csv));
}

#[test]
fn test_invalid_output_format_is_usage_error() {
    let res = Cli::try_parse_from(["velesdb", "--output", "xml", "info", "/tmp/db"]);
    let Err(err) = res else {
        panic!("xml is not a valid --output value");
    };
    assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue);
}

#[test]
fn test_global_output_coexists_with_export_output_file() {
    // `data export`'s destination file moved to `--output-file` so the global
    // `--output` format flag never collides with it (clap would panic on the
    // downcast otherwise — pinned here).
    let cli = Cli::try_parse_from([
        "velesdb",
        "--output",
        "json",
        "data",
        "export",
        "/tmp/db",
        "docs",
        "--output-file",
        "/tmp/out.json",
    ])
    .expect("global --output and export --output-file must coexist");
    assert_eq!(cli.output, Some(output::GlobalFormat::Json));
}

#[test]
fn test_global_output_coexists_with_db_snapshot_output_dir() {
    // `db snapshot`/`restore` take the destination as `--output-dir` so the
    // global `--output` format flag never collides with a PathBuf arg id.
    let cli = Cli::try_parse_from([
        "velesdb",
        "--output",
        "csv",
        "db",
        "snapshot",
        "/tmp/db",
        "--output-dir",
        "/tmp/snap",
    ])
    .expect("global --output and db snapshot --output-dir must coexist");
    assert_eq!(cli.output, Some(output::GlobalFormat::Csv));
}

#[test]
fn test_defaults_are_table_not_quiet_color() {
    let cli = Cli::try_parse_from(["velesdb", "info", "/tmp/db"]).unwrap();
    assert_eq!(cli.output, None);
    assert!(!cli.quiet);
    assert!(!cli.no_color);
}
