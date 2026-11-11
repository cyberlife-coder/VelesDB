#![allow(clippy::doc_markdown)]
#![allow(clippy::uninlined_format_args)]
//! CLI-subcommand aliases for the REPL.
//!
//! Typing a shell-shaped line (`collection list`, `query execute "SELECT
//! ..."`) inside `velesdb repl` resolves here: an exact dot-command when
//! one exists, a `VelesQL` query for `query execute`, or an actionable
//! hint naming the shell form and the closest REPL command — instead of
//! a bare `VelesQL` parse error.

// ============================================================================
// CLI-subcommand aliases
// ============================================================================

/// What a line shaped like a CLI subcommand resolves to inside the REPL.
pub(crate) enum CliLine {
    /// Run this dot-command instead — an exact REPL equivalent exists.
    DotCommand(String),
    /// Run this `VelesQL` text (e.g. `query execute "SELECT ..."`).
    Query(String),
    /// No REPL equivalent: name the shell form and the closest command
    /// instead of surfacing a bare parse error.
    Hint(String),
    /// Not a CLI subcommand — the normal `VelesQL` path applies.
    NotCli,
}

/// Maps a non-dot line to a REPL command when its first word is a CLI
/// subcommand. The match is exact-word only: `VelesQL` statements start
/// with SHOW/DESCRIBE/EXPLAIN/ANALYZE/TRUNCATE/ALTER/FLUSH/MATCH/SELECT/
/// TRAIN/CREATE/DROP/INSERT/UPSERT/UPDATE/DELETE/LET — none of which is a
/// CLI top-level name — so a real query can never land here by accident.
pub(crate) fn translate_cli_line(line: &str) -> CliLine {
    let parts: Vec<&str> = line.split_whitespace().collect();
    let Some(first) = parts.first() else {
        return CliLine::NotCli;
    };
    let rest = &parts[1..];
    match first.to_lowercase().as_str() {
        "collection" => translate_collection(rest),
        "data" => translate_data(rest),
        "query" => translate_query(rest),
        "graph" => translate_graph(rest),
        "index" => translate_index(rest),
        "info" => CliLine::DotCommand("\\info".to_string()),
        "agent" if !rest.is_empty() => CliLine::DotCommand(format!(".agent {}", rest.join(" "))),
        "help" => CliLine::DotCommand(".help".to_string()),
        "agent" | "license" | "simd" | "completions" | "repl" => CliLine::Hint(shell_hint(
            first,
            "type `.help` for the commands available here",
        )),
        _ => CliLine::NotCli,
    }
}

/// `collection <sub>` → `.collections` / `.describe` / `.analyze`, or a
/// hint to VelesQL `CREATE`/`DROP COLLECTION`.
fn translate_collection(rest: &[&str]) -> CliLine {
    let args = drop_leading_path(rest.get(1..).unwrap_or(&[]));
    match rest.first().map(|s| s.to_lowercase()).as_deref() {
        Some("list") => CliLine::DotCommand(".collections".to_string()),
        Some("show") => CliLine::DotCommand(format!(".describe {}", args.join(" "))),
        Some("analyze") => CliLine::DotCommand(format!(".analyze {}", args.join(" "))),
        Some(sub) => CliLine::Hint(format!(
            "`collection {sub}` is a shell command (velesdb collection {sub} ...) — \
             in the REPL, list with `.collections` or use CREATE/DROP COLLECTION VelesQL."
        )),
        None => CliLine::Hint(
            "`collection` is a CLI subcommand — `velesdb collection list` in a shell, \
             or `.collections` here."
                .to_string(),
        ),
    }
}

/// `data <sub>` → `.export` / `.scroll` / `.delete` where the REPL has an
/// equivalent, else a hint (`import`, `stream-insert` are shell-only).
fn translate_data(rest: &[&str]) -> CliLine {
    let args = drop_leading_path(rest.get(1..).unwrap_or(&[]));
    match rest.first().map(|s| s.to_lowercase()).as_deref() {
        Some("export") => CliLine::DotCommand(format!(".export {}", args.join(" "))),
        Some("scroll") => CliLine::DotCommand(format!(".scroll {}", args.join(" "))),
        Some("delete") => CliLine::DotCommand(format!(".delete {}", args.join(" "))),
        Some("upsert") => {
            CliLine::Hint("in the REPL use `.upsert <col> <id> <vector> [payload]`".to_string())
        }
        Some("get") => CliLine::Hint(
            "no single-point get in the REPL — use `SELECT * FROM <col> WHERE id = <id>`"
                .to_string(),
        ),
        _ => CliLine::Hint(
            "`data` is a CLI subcommand — `velesdb data <export|scroll|import|...>` in a \
             shell, or see `.help`."
                .to_string(),
        ),
    }
}

/// `query execute <sql>` runs the SQL; `query explain <sql>` → `.explain`;
/// the fusion searches stay shell-only (`NEAR [..]` hint).
fn translate_query(rest: &[&str]) -> CliLine {
    let args = drop_leading_path(rest.get(1..).unwrap_or(&[]));
    match rest.first().map(|s| s.to_lowercase()).as_deref() {
        Some("execute") => {
            let sql = args.join(" ");
            let sql = strip_quotes(sql.trim());
            if sql.is_empty() {
                CliLine::Hint(
                    "Usage: `query execute <VelesQL>` — or type the query directly.".to_string(),
                )
            } else {
                CliLine::Query(sql.to_string())
            }
        }
        Some("explain") => {
            let sql = args.join(" ");
            CliLine::DotCommand(format!(".explain {}", strip_quotes(sql.trim())))
        }
        Some(_) => CliLine::Hint(
            "`query search`/`batch-search` are shell commands — in the REPL use \
             `SELECT ... WHERE vector NEAR [..]` or `.explain <query>`."
                .to_string(),
        ),
        None => CliLine::Hint(
            "`query` is a CLI subcommand — type VelesQL directly, or `query execute <sql>`."
                .to_string(),
        ),
    }
}

/// `graph <sub> ...` → `.graph <sub> ...` (the REPL form drops the
/// database-path positional the CLI form carries). The only naming
/// difference between the two surfaces — CLI `get-edges` vs REPL `edges`
/// — is normalized; `doctor` is shell-only.
fn translate_graph(rest: &[&str]) -> CliLine {
    let Some(sub) = rest.first() else {
        return CliLine::Hint(
            "`graph` is a CLI subcommand — in the REPL use `.graph <sub>` (see `.graph help`)."
                .to_string(),
        );
    };
    let args = drop_leading_path(rest.get(1..).unwrap_or(&[]));
    match sub.to_lowercase().as_str() {
        "get-edges" => CliLine::DotCommand(format!(".graph edges {}", args.join(" "))),
        "doctor" => CliLine::Hint(
            "`graph doctor` is a shell command — run `velesdb graph doctor <db> <col>` in your shell."
                .to_string(),
        ),
        _ => CliLine::DotCommand(format!(".graph {} {}", sub, args.join(" "))),
    }
}

/// `index list` → `.indexes`; create/drop get pointed at their REPL forms.
fn translate_index(rest: &[&str]) -> CliLine {
    let args = drop_leading_path(rest.get(1..).unwrap_or(&[]));
    match rest.first().map(|s| s.to_lowercase()).as_deref() {
        Some("list") => CliLine::DotCommand(format!(".indexes {}", args.join(" "))),
        Some("create") => CliLine::Hint(
            "in the REPL use `.create-index <col> <field> [--type secondary|property|range]`"
                .to_string(),
        ),
        Some("drop") => {
            CliLine::Hint("in the REPL use `.drop-index <col> <label> <prop>`".to_string())
        }
        _ => CliLine::Hint(
            "`index` is a CLI subcommand — `velesdb index list <db> <col>` in a shell, \
             or `.indexes <col>` here."
                .to_string(),
        ),
    }
}

/// The hint printed for a CLI subcommand with no REPL equivalent.
fn shell_hint(first: &str, suggestion: &str) -> String {
    format!("`{first} ...` is a CLI subcommand — run `velesdb {first} ...` in your shell; {suggestion}.")
}

/// Drops the database-path positional the CLI form carries and the REPL
/// form does not (`collection show ./db docs` → `docs`). An argument
/// counts as a path when it contains a separator or is `.`/`..` — REPL
/// collection names are bare identifiers.
fn drop_leading_path<'a>(args: &'a [&'a str]) -> &'a [&'a str] {
    match args.first() {
        Some(first) if looks_like_path(first) => &args[1..],
        _ => args,
    }
}

/// `true` when `arg` looks like a filesystem path rather than a
/// collection/column identifier.
fn looks_like_path(arg: &str) -> bool {
    arg.contains('/') || arg.contains('\\') || arg == "." || arg == ".."
}

/// Strips one layer of matching surrounding quotes — CLI users quote their
/// query (`query execute "SELECT ..."`), the parser does not want them.
fn strip_quotes(text: &str) -> &str {
    text.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| text.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(text)
}

#[cfg(test)]
#[path = "repl_cli_alias_tests.rs"]
mod repl_cli_alias_tests;
