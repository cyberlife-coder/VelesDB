#![allow(clippy::doc_markdown)]
#![allow(clippy::uninlined_format_args)]
//! Daily-driver REPL meta commands.
//!
//! Covers: `\history`, `\save`, `\load`, `\watch`, `\edit`, `\copy`, plus the
//! CLI-subcommand alias layer (`collection list` → `.collections`, …) that
//! turns shell-shaped input into REPL commands or an actionable hint
//! instead of a bare `VelesQL` parse error.

use colored::Colorize;
use rustyline::history::History;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::repl::{LoopAction, ReplConfig, ReplEditor, ReplState};
use crate::repl_commands::CommandResult;

/// Deepest `\load` nesting allowed — a script that (transitively) loads
/// itself is rejected rather than overflowing the stack.
const MAX_LOAD_DEPTH: u8 = 8;

/// Intercepts the meta commands [`crate::repl_commands::handle_command`]
/// does not know. Returns `Some(result)` when the line was handled, `None`
/// so the standard dispatch sees it.
pub(crate) fn try_handle(
    state: &mut ReplState,
    rl: &mut ReplEditor,
    line: &str,
    config: &mut ReplConfig,
) -> Option<CommandResult> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    let cmd = parts.first()?.to_lowercase();
    Some(match cmd.as_str() {
        "\\history" | ".history" => cmd_history(rl),
        "\\save" | ".save" => cmd_save(state, &parts),
        "\\load" | ".load" => cmd_load(state, rl, &parts, config),
        "\\watch" | ".watch" => cmd_watch(state, rl, &parts, config),
        "\\edit" | ".edit" | "\\e" => cmd_edit(state, rl, config),
        "\\copy" | ".copy" => cmd_copy(state, &parts),
        _ => return None,
    })
}

// ============================================================================
// \history
// ============================================================================

/// `\history` — prints the line-editor history (this session's lines plus
/// whatever the persisted history file already held), oldest first.
fn cmd_history(rl: &ReplEditor) -> CommandResult {
    let history = rl.history();
    if history.len() == 0 {
        println!("(no history)\n");
        return CommandResult::Continue;
    }
    println!();
    for (i, entry) in history.iter().enumerate() {
        println!("{:>4}  {}", i + 1, entry.dimmed());
    }
    println!();
    CommandResult::Continue
}

// ============================================================================
// \save — last query to a script file, last result to JSON
// ============================================================================

/// `\save <file>` writes the last submitted query; `\save --result <file>`
/// writes the last result rows as pretty JSON.
fn cmd_save(state: &ReplState, parts: &[&str]) -> CommandResult {
    let (file, result_mode) = match parts.get(1) {
        Some(&"--result") | Some(&"--results") => (parts.get(2), true),
        other => (other, false),
    };
    let Some(file) = file else {
        println!("Usage: \\save <file> | \\save --result <file>\n");
        return CommandResult::Continue;
    };
    let content = match save_content(state, result_mode) {
        Ok(content) => content,
        Err(e) => return CommandResult::Error(e),
    };
    match std::fs::write(file, content) {
        Ok(()) => {
            println!("{} Saved to {}\n", "\u{2713}".green(), file.green());
            CommandResult::Continue
        }
        Err(e) => CommandResult::Error(format!("Cannot write {file}: {e}")),
    }
}

/// The text `\save` writes: the last query (plus a trailing newline), or
/// the last result rows serialized as pretty JSON.
fn save_content(state: &ReplState, result_mode: bool) -> Result<String, String> {
    if result_mode {
        return match &state.last_result {
            Some(result) => serde_json::to_string_pretty(&result.rows)
                .map_err(|e| format!("Failed to serialize result: {e}")),
            None => Err("No result to save — run a query first.".to_string()),
        };
    }
    match &state.last_query {
        Some(query) => Ok(format!("{query}\n")),
        None => Err("No query to save — run one first.".to_string()),
    }
}

// ============================================================================
// \load — run a script file line by line
// ============================================================================

/// `\load <file>` runs each non-empty, non-comment line of a script through
/// the same dispatcher as typed input (dot-commands and `VelesQL` alike).
fn cmd_load(
    state: &mut ReplState,
    rl: &mut ReplEditor,
    parts: &[&str],
    config: &mut ReplConfig,
) -> CommandResult {
    let Some(file) = parts.get(1) else {
        println!("Usage: \\load <file>\n");
        return CommandResult::Continue;
    };
    let source = match std::fs::read_to_string(file) {
        Ok(source) => source,
        Err(e) => return CommandResult::Error(format!("Cannot read {file}: {e}")),
    };
    run_script(state, rl, &source, config, file)
}

/// Runs script `source` line by line, guarding against nested `\load`
/// cycles. `origin` names the file (or editor) in the summary line.
fn run_script(
    state: &mut ReplState,
    rl: &mut ReplEditor,
    source: &str,
    config: &mut ReplConfig,
    origin: &str,
) -> CommandResult {
    if state.load_depth >= MAX_LOAD_DEPTH {
        return CommandResult::Error("\\load nesting too deep (script cycle?)".to_string());
    }
    state.load_depth += 1;
    let mut quit = false;
    let mut executed = 0usize;
    for line in source.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("--") {
            continue;
        }
        executed += 1;
        if crate::repl::run_line(state, rl, line, config) == LoopAction::Quit {
            quit = true;
            break;
        }
    }
    state.load_depth -= 1;
    println!("Loaded {executed} line(s) from {}\n", origin.green());
    if quit {
        CommandResult::Quit
    } else {
        CommandResult::Continue
    }
}

// ============================================================================
// \watch — re-run the last query on an interval
// ============================================================================

/// `\watch [seconds] [count]` re-runs the last query `count` times
/// (default 3, max 1000) every `seconds` (default 2, accepts fractions).
/// The count is bounded on purpose: an unbounded loop would hang a piped
/// session, and an interactive one can always type `\watch` again.
fn cmd_watch(
    state: &mut ReplState,
    rl: &mut ReplEditor,
    parts: &[&str],
    config: &mut ReplConfig,
) -> CommandResult {
    let Some(query) = state.last_query.clone() else {
        return CommandResult::Error("No query to watch — run one first.".to_string());
    };
    let (secs, count) = match parse_watch_args(parts) {
        Ok(args) => args,
        Err(e) => return CommandResult::Error(e),
    };
    for i in 0..count {
        println!("{}", format!("-- watch {}/{count} --", i + 1).dimmed());
        if crate::repl::run_line(state, rl, &query, config) == LoopAction::Quit {
            return CommandResult::Quit;
        }
        if i + 1 < count {
            std::thread::sleep(std::time::Duration::from_secs_f64(secs));
        }
    }
    CommandResult::Continue
}

/// Parses `\watch`'s optional `[seconds] [count]` arguments.
fn parse_watch_args(parts: &[&str]) -> Result<(f64, usize), String> {
    let secs = match parts.get(1) {
        Some(raw) => raw
            .parse::<f64>()
            .ok()
            .filter(|v| *v >= 0.0)
            .ok_or_else(|| format!("Invalid interval: {raw}"))?,
        None => 2.0,
    };
    let count = match parts.get(2) {
        Some(raw) => raw
            .parse::<usize>()
            .ok()
            .filter(|v| (1..=1000).contains(v))
            .ok_or_else(|| format!("Invalid count (1-1000): {raw}"))?,
        None => 3,
    };
    Ok((secs, count))
}

// ============================================================================
// \edit — re-run the last query through $VISUAL/$EDITOR
// ============================================================================

/// `\edit` writes the last query to a scratch file, opens it in
/// `$VISUAL`/`$EDITOR`, then runs the edited lines like `\load` does.
fn cmd_edit(state: &mut ReplState, rl: &mut ReplEditor, config: &mut ReplConfig) -> CommandResult {
    let Some(query) = state.last_query.clone() else {
        return CommandResult::Error("No query to edit — run one first.".to_string());
    };
    let Some(editor) = std::env::var_os("VISUAL")
        .or_else(|| std::env::var_os("EDITOR"))
        .and_then(|value| value.into_string().ok())
    else {
        return CommandResult::Error("No editor configured — set $VISUAL or $EDITOR.".to_string());
    };
    let file = edit_scratch_path();
    if let Err(e) = std::fs::write(&file, format!("{query}\n")) {
        return CommandResult::Error(format!("Cannot write scratch file: {e}"));
    }
    let status = spawn_editor(&editor, &file);
    let source = std::fs::read_to_string(&file);
    let _ = std::fs::remove_file(&file);
    if let Err(e) = status {
        return CommandResult::Error(e);
    }
    match source {
        Ok(source) => run_script(state, rl, &source, config, "edited query"),
        Err(e) => CommandResult::Error(format!("Cannot read edited file: {e}")),
    }
}

/// Unique scratch file for `\edit` (pid + timestamp — no tempfile dep for
/// one file). It lives in the user-private data directory, next to the
/// persisted history file — not in the shared `env::temp_dir`, where a
/// predictable path would make the file (which holds a query) an
/// InsecureStorage finding.
fn edit_scratch_path() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(format!("velesdb_edit_{}_{nanos}.sql", std::process::id()))
}

/// Runs `$VISUAL`/`$EDITOR` on `file` and waits for it to exit. Unix
/// honours the convention that the variable may hold a shell command
/// (`"code --wait"`); Windows goes through `cmd /C`.
fn spawn_editor(editor: &str, file: &Path) -> Result<(), String> {
    #[cfg(windows)]
    let status = std::process::Command::new("cmd")
        .args(["/C", &format!("{editor} \"{}\"", file.display())])
        .status();
    #[cfg(not(windows))]
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$@\""))
        .arg(editor)
        .arg(file)
        .status();
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("Editor exited with {s}")),
        Err(e) => Err(format!("Cannot launch editor '{editor}': {e}")),
    }
}

// ============================================================================
// \copy — last result (or query) to the system clipboard
// ============================================================================

/// `\copy` copies the last result rows as JSON; `\copy query` copies the
/// last query text. When no clipboard tool exists the payload is printed
/// instead — a piped or headless session still gets the data.
fn cmd_copy(state: &ReplState, parts: &[&str]) -> CommandResult {
    let (label, text) = match copy_payload(state, parts) {
        Ok(payload) => payload,
        Err(e) => return CommandResult::Error(e),
    };
    match clipboard_copy(&text) {
        Ok(()) => println!("{} Copied {label} to clipboard.\n", "\u{2713}".green()),
        Err(reason) => {
            println!(
                "{}",
                format!("(clipboard unavailable: {reason} — printing instead)").dimmed()
            );
            println!("{text}\n");
        }
    }
    CommandResult::Continue
}

/// What `\copy` copies: `query` → last query text; anything else → last
/// result rows as pretty JSON.
fn copy_payload(state: &ReplState, parts: &[&str]) -> Result<(String, String), String> {
    match parts.get(1).map(|s| s.to_lowercase()).as_deref() {
        Some("query") => match &state.last_query {
            Some(query) => Ok(("query".to_string(), query.clone())),
            None => Err("No query to copy — run one first.".to_string()),
        },
        _ => match &state.last_result {
            Some(result) => serde_json::to_string_pretty(&result.rows)
                .map(|json| (format!("{} row(s)", result.rows.len()), json))
                .map_err(|e| format!("Failed to serialize result: {e}")),
            None => Err("No result to copy — run a query first.".to_string()),
        },
    }
}

/// Writes `text` to the system clipboard through the platform tool
/// (`pbcopy`, `clip`, `wl-copy`, `xclip`, `xsel`), so no clipboard crate is
/// pulled in for a single command. `Err` names why nothing was copied.
fn clipboard_copy(text: &str) -> Result<(), String> {
    for (program, args) in clipboard_tools() {
        if pipe_to(program, args, text) {
            return Ok(());
        }
    }
    Err(clipboard_hint().to_string())
}

/// Clipboard programs tried in order, per platform.
#[cfg(target_os = "macos")]
fn clipboard_tools() -> &'static [(&'static str, &'static [&'static str])] {
    &[("pbcopy", &[])]
}

/// Clipboard programs tried in order, per platform.
#[cfg(windows)]
fn clipboard_tools() -> &'static [(&'static str, &'static [&'static str])] {
    &[("clip", &[])]
}

/// Clipboard programs tried in order, per platform.
#[cfg(all(unix, not(target_os = "macos")))]
fn clipboard_tools() -> &'static [(&'static str, &'static [&'static str])] {
    &[
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ]
}

/// Clipboard programs tried in order, per platform.
#[cfg(not(any(unix, windows)))]
fn clipboard_tools() -> &'static [(&'static str, &'static [&'static str])] {
    &[]
}

/// The reason returned when every clipboard tool failed.
fn clipboard_hint() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "pbcopy not available"
    }
    #[cfg(windows)]
    {
        "clip not available"
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        "no clipboard tool found (install wl-copy, xclip or xsel)"
    }
    #[cfg(not(any(unix, windows)))]
    {
        "no clipboard tool on this platform"
    }
}

/// Pipes `text` to `program`'s stdin; `true` when the tool exists and
/// exits successfully. Spawn/write/wait failures all count as failure so
/// the caller tries the next tool.
fn pipe_to(program: &str, args: &[&str], text: &str) -> bool {
    let Ok(mut child) = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }
    child.wait().map(|status| status.success()).unwrap_or(false)
}

#[cfg(test)]
#[path = "repl_meta_cmds_tests.rs"]
mod repl_meta_cmds_tests;
