//! `velesdb config` — the engine configuration the CLI loads.
//!
//! Every action resolves its file through [`ConfigSource`], the resolution
//! [`helpers::open_database`] uses: `--config`/`VELESDB_CONFIG`, else
//! `./velesdb.toml`, else the core defaults, with the `VELESDB_*` engine
//! variables on top. What `config show` prints is therefore what the next
//! database-opening command applies, not a second reading of the file.

use std::path::Path;

use anyhow::{bail, Context, Result};
use velesdb_core::config::VelesConfig;

use crate::commands::{ConfigAction, ConfigFormat};
use crate::helpers::{self, ConfigSource, DEFAULT_CONFIG_FILE};

/// Dispatches a `velesdb config` action.
pub fn handle_config(action: ConfigAction) -> Result<()> {
    let default_file = Path::new(DEFAULT_CONFIG_FILE);
    match action {
        ConfigAction::Path => {
            print_path(&ConfigSource::resolve(
                helpers::config_path(),
                default_file,
            )?);
            Ok(())
        }
        ConfigAction::Show { format } => {
            let source = ConfigSource::resolve(helpers::config_path(), default_file)?;
            println!("{}", render(&source.load()?, format)?);
            Ok(())
        }
        ConfigAction::Validate { file } => {
            let explicit = file.as_deref().or(helpers::config_path());
            validate(&ConfigSource::resolve(explicit, default_file)?)
        }
        ConfigAction::Edit => edit(&ConfigSource::resolve(
            helpers::config_path(),
            default_file,
        )?),
    }
}

/// Prints the file on stdout alone, so `$(velesdb config path)` is usable;
/// the no-file case goes to stderr and leaves stdout empty.
fn print_path(source: &ConfigSource<'_>) {
    match source.file() {
        Some(file) => println!("{}", display_path(file)),
        None => eprintln!(
            "no config file: core defaults + VELESDB_* engine variables apply \
             (pass --config <file> or create ./{DEFAULT_CONFIG_FILE})"
        ),
    }
}

fn display_path(file: &Path) -> String {
    std::fs::canonicalize(file)
        .unwrap_or_else(|_| file.to_path_buf())
        .display()
        .to_string()
}

/// The engine tables of `config`. `server` and `logging` are fields of
/// [`VelesConfig`] too, but the engine-only loaders never read them
/// ([`VelesConfig::is_engine_table`]), so printing their defaults would show
/// values nothing applies.
pub(crate) fn engine_tables(config: &VelesConfig) -> Result<toml::Table> {
    let toml::Value::Table(mut table) =
        toml::Value::try_from(config).context("failed to serialize the engine config")?
    else {
        bail!("the engine config did not serialize to a TOML table");
    };
    table.retain(|name, _| VelesConfig::is_engine_table(name));
    Ok(table)
}

fn render(config: &VelesConfig, format: ConfigFormat) -> Result<String> {
    let table = engine_tables(config)?;
    Ok(match format {
        ConfigFormat::Toml => toml::to_string_pretty(&table)?,
        ConfigFormat::Json => serde_json::to_string_pretty(&table)?,
    })
}

/// Loads `source` through the same loader `open_database` uses — an invalid
/// file is an `Err`, hence exit code 1 — then reports which tables of the file
/// that loader read and which it did not.
fn validate(source: &ConfigSource<'_>) -> Result<()> {
    source.load()?;
    let Some(file) = source.file() else {
        println!("OK: no config file; core defaults + VELESDB_* engine variables are valid");
        return Ok(());
    };
    let (engine, other) = split_tables(file)?;
    println!(
        "OK: {} (engine tables read: {}; VELESDB_* engine variables applied)",
        display_path(file),
        bracketed(&engine, "none")
    );
    if !other.is_empty() {
        println!(
            "not read by the CLI engine: {} (velesdb-server reads its own tables itself)",
            bracketed(&other, "")
        );
    }
    Ok(())
}

fn split_tables(file: &Path) -> Result<(Vec<String>, Vec<String>)> {
    let raw = std::fs::read_to_string(file)
        .with_context(|| format!("failed to read {}", file.display()))?;
    let doc: toml::Table = toml::from_str(&raw)?;
    Ok(doc
        .keys()
        .cloned()
        .partition(|name| VelesConfig::is_engine_table(name)))
}

fn bracketed(names: &[String], empty: &str) -> String {
    if names.is_empty() {
        return empty.to_owned();
    }
    names
        .iter()
        .map(|n| format!("[{n}]"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Opens the resolved file in `$VISUAL`/`$EDITOR` (split on whitespace, run
/// without a shell), then validates what was saved.
fn edit(source: &ConfigSource<'_>) -> Result<()> {
    let Some(file) = source.file() else {
        bail!("no config file to edit: pass --config <file> or create ./{DEFAULT_CONFIG_FILE}");
    };
    let editor = editor_command()?;
    let Some((program, args)) = editor.split_first() else {
        bail!("set $VISUAL or $EDITOR to the editor to run");
    };
    let status = std::process::Command::new(program)
        .args(args)
        .arg(file)
        .status()
        .with_context(|| format!("failed to launch editor `{program}`"))?;
    if !status.success() {
        bail!("editor `{program}` exited with {status}; the file was not re-validated");
    }
    validate(source)
}

fn editor_command() -> Result<Vec<String>> {
    ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(|var| std::env::var(var).ok())
        .map(|value| {
            value
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .find(|words| !words.is_empty())
        .context("set $VISUAL or $EDITOR to the editor to run")
}
