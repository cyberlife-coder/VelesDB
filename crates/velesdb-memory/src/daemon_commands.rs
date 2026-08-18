//! Early subcommands that never open the live store as a daemon:
//! `export`, `migrate-embeddings` / `migrate-rebuild`, and `compile-stdin`.

use crate::backends::{build_embedder, ConfiguredEmbedder};
use crate::startup::{apply_config_file, default_store_path};

/// Run `export`: write every live fact of the store as JSONL, embedder-free.
///
/// Flags: `--output <path>` (default: stdout), `--include-internal` (list
/// graph scaffolding and reserved keys verbatim — the backup shape),
/// `--store <path>` (default: `VELESDB_MEMORY_PATH`, then the config file,
/// then the standard location — same precedence as the daemon).
///
/// # Errors
/// An unparsable invocation, a store that cannot be opened (a RUNNING daemon
/// holds its single-writer lock — stop it first; the refusal says so), or a
/// write failure on the output.
pub(crate) fn run_export(
    argv: &[String],
    flags: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    apply_config_file(argv)?;
    let options = ExportOptions::parse(flags)?;
    let store = options
        .store_path
        .or_else(|| std::env::var("VELESDB_MEMORY_PATH").ok())
        .unwrap_or_else(default_store_path);
    let store = std::path::Path::new(&store);
    let written = write_export(store, options.output.as_deref(), options.include_internal)?;
    eprintln!("[velesdb-memory] exported {written} memories");
    Ok(())
}

/// The destination half of [`run_export`]: a named file (buffered, flushed)
/// or stdout (locked — on this subcommand stdout carries data, not MCP).
pub(crate) fn write_export(
    store: &std::path::Path,
    output: Option<&str>,
    include_internal: bool,
) -> Result<u64, Box<dyn std::error::Error>> {
    if let Some(path) = output {
        let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
        let written = velesdb_memory::export::export_jsonl(store, &mut file, include_internal)?;
        std::io::Write::flush(&mut file)?;
        Ok(written)
    } else {
        let stdout = std::io::stdout();
        let mut lock = stdout.lock();
        Ok(velesdb_memory::export::export_jsonl(
            store,
            &mut lock,
            include_internal,
        )?)
    }
}

/// The `export` subcommand's parsed flags — split from [`run_export`] so
/// each function carries one concern: this one the CLI grammar, that one
/// the walk.
pub(crate) struct ExportOptions {
    store_path: Option<String>,
    output: Option<String>,
    include_internal: bool,
}

impl ExportOptions {
    fn parse(flags: &[String]) -> Result<Self, Box<dyn std::error::Error>> {
        let mut options = Self {
            store_path: None,
            output: None,
            include_internal: false,
        };
        let mut it = flags.iter();
        while let Some(flag) = it.next() {
            match flag.as_str() {
                "--include-internal" => options.include_internal = true,
                "--store" => options.store_path = Some(Self::value_of(&mut it, "--store")?),
                "--output" => options.output = Some(Self::value_of(&mut it, "--output")?),
                other => return Err(format!("unknown export flag '{other}'").into()),
            }
        }
        Ok(options)
    }

    /// The argument a valued flag requires, or the error naming the flag.
    fn value_of(
        it: &mut std::slice::Iter<'_, String>,
        flag: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        Ok(it
            .next()
            .ok_or_else(|| format!("{flag} requires a path argument"))?
            .clone())
    }
}

/// Run `migrate-embeddings`: diagnose a store against the configured target
/// embedder and print the regime it resolves to.
///
/// `argv` is the whole command line, so `--config` keeps working here exactly
/// as it does for the daemon; `flags` is what follows the subcommand.
///
/// Exits `2` on a refusal rather than returning `Ok`. A command that printed
/// `REFUSE` and exited `0` would be read as success by every script wrapping
/// it, and the whole point of the refusal is that something must not proceed.
///
/// # Errors
/// An unparsable invocation, a non-dry-run request without `--destination`,
/// an unreachable embedder, a store that cannot be read or copied, or any
/// stage of the migration itself refusing.
pub(crate) fn run_migrate_embeddings(
    argv: &[String],
    flags: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    use velesdb_memory::migration;

    let options = migration::parse_migrate_args(flags)?;

    apply_config_file(argv)?;
    let store_path = migrate_store_path(&options);
    // The target's identity comes from the embedder the daemon WOULD build, not
    // from a flag: a model name an operator typed is a claim, and the dimension
    // has to be the one the embedder actually produces.
    let ConfiguredEmbedder { embedder, model } = build_embedder()?;
    let target = migration::TargetContract {
        model,
        dimension: embedder.dimension(),
        strategy: options.strategy,
    };
    let scratch = migrate_scratch_parent(&options, &store_path)?;

    if options.dry_run {
        let report = migration::dry_run(
            &store_path,
            &scratch,
            &target,
            options.destination.as_deref(),
        )?;
        print!("{}", migration::render(&report));
        if migration::refuses(&report) {
            std::process::exit(2);
        }
        return Ok(());
    }
    run_migrate_rebuild(&options, &store_path, &scratch, &target, embedder.as_ref())
}

/// The non-dry-run tail of `migrate-embeddings`: rebuild, validate, switch.
///
/// The chain enters wherever the journal stands — a re-run after any crash
/// resumes rather than failing on the stage the crash already completed —
/// and each stage that ran reports itself; one that was journalled as done
/// stays silent rather than misreporting work this run did not do.
pub(crate) fn run_migrate_rebuild(
    options: &velesdb_memory::migration::MigrateOptions,
    store_path: &std::path::Path,
    scratch: &std::path::Path,
    target: &velesdb_memory::migration::TargetContract,
    embedder: &dyn velesdb_memory::Embedder,
) -> Result<(), Box<dyn std::error::Error>> {
    use velesdb_memory::migration;

    let destination = migration::require_destination(options)?;
    let outcome = migration::migrate(
        store_path,
        scratch,
        target,
        &destination,
        embedder,
        MIGRATE_BATCH,
    )?;
    if let Some(executed) = &outcome.executed {
        print!("{}", migration::render(&executed.report));
        println!(
            "rebuild: {} facts written, {} already present, {} edges, journal at {}",
            executed.rebuild.facts,
            executed.rebuild.collisions,
            executed.rebuild.edges,
            executed.workspace.display(),
        );
    }
    if let Some(validated) = &outcome.validated {
        println!(
            "validated: {} facts and {} edges compared, {} divergence(s) explained by expiry",
            validated.facts, validated.edges, validated.explained_by_expiry,
        );
    }
    println!("activated: {}", outcome.switched.activated.display());
    println!("{}", migration::migration_complete_notice());
    Ok(())
}

/// The rebuild's batch size: the fact export's own proven default.
pub(crate) const MIGRATE_BATCH: usize = 1024;

/// The store `migrate-embeddings` operates on: `--store`, else exactly where
/// the daemon would look (`VELESDB_MEMORY_PATH`, else the advertised default).
pub(crate) fn migrate_store_path(
    options: &velesdb_memory::migration::MigrateOptions,
) -> std::path::PathBuf {
    options.store.clone().unwrap_or_else(|| {
        std::path::PathBuf::from(
            std::env::var("VELESDB_MEMORY_PATH").unwrap_or_else(|_| default_store_path()),
        )
    })
}

/// Where the diagnosis stages its verified copy: `--scratch`, else beside the
/// store — see `migration::default_scratch_parent` for why not the temp dir.
///
/// # Errors
/// The store path has no usable parent and `--scratch` was not given.
pub(crate) fn migrate_scratch_parent(
    options: &velesdb_memory::migration::MigrateOptions,
    store_path: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    if let Some(dir) = options.scratch.clone() {
        return Ok(dir);
    }
    // Canonicalize first so a relative store path ("./store") yields a real
    // parent rather than the empty string. A store that does not exist falls
    // through unchanged — the diagnosis then fails on it with its own, more
    // precise message.
    let resolved = std::fs::canonicalize(store_path).unwrap_or_else(|_| store_path.to_path_buf());
    velesdb_memory::migration::default_scratch_parent(&resolved)
}

/// Default token budget of `compile-stdin` when `--budget` is omitted.
///
/// Sized for the job the hook does: a tool result big enough to be worth
/// compiling, compressed to something an agent can still read in full.
// Tous ses consommateurs sont gates sur `context` : sans cette feature la
// constante est reellement morte, et -D warnings en fait une erreur.
#[cfg(feature = "context")]
pub(crate) const DEFAULT_COMPILE_STDIN_BUDGET: u64 = 2_000;

/// Parsed `compile-stdin` invocation.
#[cfg(feature = "context")]
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CompileStdinOptions {
    token_budget: u64,
    query: String,
}

#[cfg(feature = "context")]
impl Default for CompileStdinOptions {
    fn default() -> Self {
        Self {
            token_budget: DEFAULT_COMPILE_STDIN_BUDGET,
            query: String::new(),
        }
    }
}

/// What `compile-stdin` writes to stdout: one JSON object, so a shell hook
/// gets the compiled text AND the accounting from a single stream (`jq` is
/// already a hard requirement of the hooks).
#[cfg(feature = "context")]
#[derive(serde::Serialize)]
pub(crate) struct CompileStdinOutput {
    content: String,
    tokens_in: u64,
    tokens_out: u64,
    tokens_saved: u64,
    risk: String,
}

/// Parse `compile-stdin`'s flags. Hand-rolled for the same reason as
/// `--version`/`--http` above: two flags do not justify a `clap` dependency
/// in the shipped binary.
///
/// # Errors
/// A message naming the offending flag when it is unknown, when its value is
/// missing, or when `--budget` is not a positive integer.
/// Validate `--budget`'s value. Split out of [`parse_compile_stdin_args`] to
/// keep that loop's branching within the repo's complexity ceiling.
///
/// # Errors
/// When the value is absent, not an integer, or zero — a zero budget fits no
/// fragment at all, so it can only ever produce the empty-compilation failure
/// [`compile_stdin_json`] rejects anyway.
#[cfg(feature = "context")]
pub(crate) fn parse_compile_stdin_budget(value: Option<&String>) -> Result<u64, String> {
    let raw = value.ok_or_else(|| "--budget requires a value".to_owned())?;
    let parsed: u64 = raw
        .parse()
        .map_err(|_| format!("--budget expects a positive integer, got {raw:?}"))?;
    if parsed == 0 {
        return Err("--budget must be greater than 0".to_owned());
    }
    Ok(parsed)
}

#[cfg(feature = "context")]
pub(crate) fn parse_compile_stdin_args(args: &[String]) -> Result<CompileStdinOptions, String> {
    let mut options = CompileStdinOptions::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = args.get(index + 1);
        match flag {
            "--budget" => {
                options.token_budget = parse_compile_stdin_budget(value)?;
                index += 2;
            }
            "--query" => {
                options
                    .query
                    .clone_from(value.ok_or_else(|| "--query requires a value".to_owned())?);
                index += 2;
            }
            other => return Err(format!("unknown compile-stdin flag {other:?}")),
        }
    }
    Ok(options)
}

/// Compile `text` under `options` and render the JSON payload.
///
/// # Errors
/// When `text` is empty, when segmentation hits a [`velesdb_memory::limits`]
/// cap, or when the budget leaves no room for any context.
#[cfg(feature = "context")]
pub(crate) fn compile_stdin_json(
    text: &str,
    options: &CompileStdinOptions,
) -> Result<String, Box<dyn std::error::Error>> {
    use velesdb_memory::context::{
        segment_transcript, CompilePolicy, CompileRequest, ContextCompiler, SegmentationPolicy,
    };

    if text.trim().is_empty() {
        return Err("compile-stdin received empty input on stdin".into());
    }

    let outcome = segment_transcript(text, &SegmentationPolicy::default())?;
    let request = CompileRequest {
        query: options.query.clone(),
        fragments: outcome
            .segments
            .into_iter()
            .map(|segment| segment.fragment)
            .collect(),
        project: None,
        target_model: None,
        token_budget: options.token_budget,
        memory_scope: None,
        policy: None,
    };
    let compiled = ContextCompiler::new(CompilePolicy::default()).compile(&request)?;

    // The compiler externalizes rather than truncates: when no single
    // fragment fits, everything moves behind a retrieval handle and the
    // assembled content is empty. That is a legitimate compilation, but a
    // useless one to return as a *replacement* for real content — surface it
    // as an error so the caller keeps the original instead of shipping an
    // empty string.
    if compiled.content.is_empty() {
        return Err(format!(
            "a budget of {} tokens fits none of the {} input tokens — every fragment was \
             externalized and the compiled context is empty; raise --budget",
            options.token_budget, compiled.insights.tokens_in
        )
        .into());
    }

    let output = CompileStdinOutput {
        content: compiled.content,
        tokens_in: compiled.insights.tokens_in,
        tokens_out: compiled.insights.tokens_out,
        tokens_saved: compiled.insights.tokens_saved,
        risk: format!("{:?}", compiled.risk).to_lowercase(),
    };
    Ok(serde_json::to_string(&output)?)
}

/// Read stdin, compile it, print the JSON payload.
///
/// # Errors
/// Propagates flag-parsing, stdin-read, and compilation failures.
#[cfg(feature = "context")]
pub(crate) fn run_compile_stdin(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Read as _;

    let options = parse_compile_stdin_args(args)?;
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    println!("{}", compile_stdin_json(&text, &options)?);
    Ok(())
}

#[cfg(not(feature = "context"))]
pub(crate) fn run_compile_stdin(_args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    Err("`compile-stdin` requires building with `--features context`".into())
}

#[cfg(all(test, feature = "context"))]
mod compile_stdin_tests {
    use super::{
        compile_stdin_json, parse_compile_stdin_args, CompileStdinOptions,
        DEFAULT_COMPILE_STDIN_BUDGET,
    };

    /// A tool-output-shaped corpus: repetitive log lines, the exact case a
    /// `PostToolUse` hook has to shrink.
    fn noisy_tool_output() -> String {
        use std::fmt::Write as _;

        let mut text = String::new();
        for i in 0..120 {
            let _ = writeln!(
                text,
                "[2026-07-25T01:0{}:00Z] INFO  worker: processing batch {} of 120 — retry=0 status=ok",
                i % 10,
                i
            );
        }
        text
    }

    fn parse(value: &str) -> serde_json::Value {
        serde_json::from_str(value).expect("compile-stdin must emit valid JSON")
    }

    #[test]
    fn tight_budget_actually_shrinks_the_payload() {
        let options = CompileStdinOptions {
            token_budget: 1_500,
            query: "what did the worker do".to_owned(),
        };
        let compiled = parse(&compile_stdin_json(&noisy_tool_output(), &options).unwrap());

        let tokens_in = compiled["tokens_in"].as_u64().unwrap();
        let tokens_out = compiled["tokens_out"].as_u64().unwrap();
        assert!(tokens_in > 0, "tokens_in must be measured, got {tokens_in}");
        assert!(
            tokens_out < tokens_in,
            "a 200-token budget over {tokens_in} tokens of logs must compress: got {tokens_out}"
        );
        assert_eq!(
            compiled["tokens_saved"].as_u64().unwrap(),
            tokens_in - tokens_out
        );
        let content = compiled["content"].as_str().unwrap();
        assert!(
            !content.is_empty(),
            "an empty compilation is worse than no compilation — the caller would replace a \
             real tool result with nothing"
        );
        assert!(
            content.len() < noisy_tool_output().len(),
            "the compiled content must be shorter than the raw tool output"
        );
    }

    /// A budget too small to fit even one fragment makes the compiler
    /// externalize everything and emit an EMPTY context. Returning that as a
    /// success is a trap: `compile-stdin`'s caller (a `PostToolUse` hook) would
    /// swap a real tool result for an empty string. Fail loudly instead, so
    /// the caller falls back to the untouched output.
    #[test]
    fn budget_too_small_for_any_fragment_is_an_error() {
        let options = CompileStdinOptions {
            token_budget: 50,
            query: String::new(),
        };
        let error = compile_stdin_json(&noisy_tool_output(), &options).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("budget"),
            "the error must point at the budget, got {message}"
        );
    }

    #[test]
    fn compilation_is_byte_identical_across_runs() {
        let options = CompileStdinOptions {
            token_budget: 1_500,
            query: "worker batches".to_owned(),
        };
        let first = compile_stdin_json(&noisy_tool_output(), &options).unwrap();
        let second = compile_stdin_json(&noisy_tool_output(), &options).unwrap();
        assert_eq!(first, second, "the compiler must be deterministic");
    }

    #[test]
    fn empty_stdin_is_rejected() {
        let error = compile_stdin_json("   \n\t ", &CompileStdinOptions::default()).unwrap_err();
        assert!(
            error.to_string().contains("empty"),
            "the error must name the cause, got {error}"
        );
    }

    #[test]
    fn flags_default_and_override() {
        assert_eq!(
            parse_compile_stdin_args(&[]).unwrap(),
            CompileStdinOptions {
                token_budget: DEFAULT_COMPILE_STDIN_BUDGET,
                query: String::new(),
            }
        );
        let parsed = parse_compile_stdin_args(&[
            "--budget".to_owned(),
            "512".to_owned(),
            "--query".to_owned(),
            "why did it fail".to_owned(),
        ])
        .unwrap();
        assert_eq!(parsed.token_budget, 512);
        assert_eq!(parsed.query, "why did it fail");
    }

    #[test]
    fn malformed_flags_are_rejected() {
        for bad in [
            vec!["--budget".to_owned()],
            vec!["--budget".to_owned(), "zero".to_owned()],
            vec!["--budget".to_owned(), "0".to_owned()],
            vec!["--nope".to_owned()],
        ] {
            assert!(
                parse_compile_stdin_args(&bad).is_err(),
                "must reject {bad:?}"
            );
        }
    }
}
