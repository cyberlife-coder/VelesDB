# ISSUE-2726 — cli-data evidence

## Explore (2026-11-10)

Issue: `data export` JSON-only, no format flag, no row filter. `data import`
no --dry-run / --on-conflict / --continue-on-error + error report.

Findings (before coding):
- `handlers::data::handle_export` materializes the whole collection into
  `Vec<Value>` (`collect_export_records`). Replacing with streaming
  `scroll_batch` keeps memory bounded.
- `VectorCollection::scroll_batch(cursor, n, Option<&Filter>)` already takes
  a filter — the WHERE plumbing exists.
- `velesql::Condition -> filter::Condition` (`From`) maps vector search /
  similarity / graph match to an ALWAYS-TRUE sentinel — feeding a --where
  clause straight through it would make `similarity(...) > x` match every
  row. `filter::conversion::inexpressible_scope_construct` is the existing
  fail-closed checker (built for observer scopes); wrapped it in the public
  `Filter::try_from_where` rather than copy the fail-closed set.
- Import: default today = silent skip+count, exit 0. `--continue-on-error`
  implies fail-fast default; central asked for advice → fail-fast confirmed
  (same class as #2273 breaking-refusal; CHANGELOG BREAKING entry planned).

## Plan

Two sequential PRs (issue asks "PR petites et sequentielles"):
- PR1 (this branch feat/2726-data-export-format): core `Filter::try_from_where`
  + CLI `data export --format json|jsonl|csv` + `--where` + docs.
- PR2: import `--dry-run` / `--on-conflict` / `--continue-on-error` + error
  report + fail-fast default (BREAKING note).

## Implement PR1 — evidence

Files:
- crates/velesdb-core/src/filter/conversion.rs — `Filter::try_from_where`
  (ungates `inexpressible_scope_construct`, same check the observer scope
  uses); +2 tests in conversion_tests.rs.
- crates/velesdb-cli/src/export.rs — streaming writers (json/jsonl/csv) over
  scroll_batch pages; `parse_where_filter` wraps the clause in
  `SELECT * FROM export WHERE …` and requires the parsed Query to equal the
  bare SELECT minus where_clause (rejects LIMIT/ORDER BY/second statements).
- commands.rs/cli_types.rs/main.rs — `--format`, `--where`, `ExportFormatArg`.
- scripts/durability-barrier-baseline.txt — export.rs exempted (same contract
  as velesdb-memory daemon_commands: caller-owned one-shot output file,
  not acknowledged DB state).

Checks (all run, outputs above in transcript):
- `cargo test -p velesdb-cli --bin velesdb -- --test-threads=1`: 273/273 ok
  (9 new: where lowering, smuggled LIMIT/ORDER BY/similarity refused, JSON
  byte-shape, JSONL lines, CSV round-trip through import_csv, empty coll).
- `cargo test -p velesdb-core … try_from_where`: 2/2 ok.
- Clippy workspace pass1 (persistence,gpu,update-check,openapi,test-fault-injection,
  pedantic): clean. Pass2 (persistence,update-check,internal-bench): clean.
- local-ci lint+hygiene replay: 46 gates, 1 fail → durability-barrier →
  fixed by baseline exemption, re-run PASSED.
- check-doc-contract, check-todo-annotations, check-promise-contract: PASSED.

## Implement PR2 — evidence (feat/2726-data-import-flags)

Files:
- crates/velesdb-cli/src/import_validate.rs (new) — pre-write validation
  pass: validate_jsonl/validate_csv stream the file once collecting ids +
  error ordinals (all errors under --dry-run/--continue-on-error, else
  stops at first), resolve_conflicts (--on-conflict), existing_ids chunked
  probes, open_error_report/record_error streaming {line,reason} JSONL
  (write errors propagate — a silently truncated report is worse than a
  loud abort), dry_run_stats, take_conflict (skip mode also dedups ids
  seen earlier in the same file — first wins), filter_conflicting (VRB1).
- crates/velesdb-cli/src/import.rs — OnConflict{Upsert,Skip,Fail},
  ImportConfig{dry_run,on_conflict,continue_on_error,error_report},
  ImportStats{+skipped,+conflicts,+error_details}; fail-fast default:
  validation runs before get_or_create_collection, so a bad file never
  creates/touches the collection (BREAKING vs old silent-skip).
- commands.rs/cli_types.rs/main.rs/handlers/data.rs — flag plumbing +
  extended summary (total/imported/skipped/conflicts/errors + details).
- Split: import.rs+validate kept <500 NLOC each (361/342).
- scripts/durability-barrier-baseline.txt — import_validate.rs exempted
  (--error-report = caller-owned diagnostic artifact, same as export.rs).

Checks:
- cargo test -p velesdb-cli --bin velesdb -- --test-threads=1: 270/270 ok
  (28 import tests incl. fail-fast-no-write, dry-run, skip/fail conflicts,
  continue-on-error + report contents).
- clippy workspace pedantic pass: clean. durability-barrier: PASSED.
- Pre-commit (fmt/clippy/unsafe/SAFETY/TODO/secrets + workspace tests):
  PASSED on 2nd attempt — 1st run hit unrelated flake
  an_arena_reserves_its_blocks_rather_than_leaving_holes (HNSW arena,
  concurrent build contention; passes in isolation, unrelated to diff).

## Merge log
- PR1 #2734: CI 68 pass / 3 skipping, Codacy pass, 0 review comments;
  rebased onto develop (CHANGELOG conflict vs #2725 entry only),
  force-with-lease push 867a939fb.
