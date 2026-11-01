//! Tests for the collection lock audit and for `crate::pools`, the one module
//! that submits work to a rayon pool: each half of the rule panics when
//! broken, and no other module can reach a pool.

use super::{assert_no_guard_held, guards_held, Mutex, RwLock};
use std::panic::{catch_unwind, AssertUnwindSafe};

fn one_thread_pool() -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .expect("test: rayon pool")
}

#[test]
fn a_collection_lock_taken_on_a_rayon_worker_panics() {
    let lock = RwLock::new(0_u32);
    let mutex = Mutex::new(0_u32);
    let pool = one_thread_pool();

    let read = pool.install(|| catch_unwind(AssertUnwindSafe(|| *lock.read())));
    let write = pool.install(|| catch_unwind(AssertUnwindSafe(|| *lock.write())));
    let locked = pool.install(|| catch_unwind(AssertUnwindSafe(|| *mutex.lock())));

    assert!(read.is_err(), "a read guard was taken on a rayon worker");
    assert!(write.is_err(), "a write guard was taken on a rayon worker");
    assert!(locked.is_err(), "a mutex guard was taken on a rayon worker");
    assert_eq!(*lock.read(), 0, "off the pool the lock is usable");
}

/// An application's own rayon jobs may call into a collection.
#[test]
fn an_application_job_may_take_a_collection_lock() {
    let lock = RwLock::new(7_u32);
    let read = crate::pools::application_pool(1).install(|| *lock.read());
    assert_eq!(read, 7);
}

#[test]
fn a_guard_held_across_a_parallel_submission_panics() {
    let lock = RwLock::new(vec![1_u32, 2, 3]);
    let mutex = Mutex::new(0_u32);

    let read = lock.read();
    assert_eq!(guards_held(), 1);
    assert!(
        catch_unwind(AssertUnwindSafe(|| crate::pools::map(&read, |x| x + 1))).is_err(),
        "a parallel submission was let through with a read guard held"
    );
    drop(read);
    let locked = mutex.lock();
    assert!(
        catch_unwind(assert_no_guard_held).is_err(),
        "the assert let a mutex guard through"
    );
    drop(locked);
    assert_eq!(guards_held(), 0, "a dropped guard is no longer counted");
    assert_eq!(crate::pools::map(&[1_u32, 2], |x| x + 1), vec![2, 3]);
}

#[test]
fn the_count_is_per_thread() {
    let lock = RwLock::new(0_u32);
    let _held = lock.write();

    std::thread::scope(|s| {
        s.spawn(|| {
            assert_eq!(guards_held(), 0, "another thread's guard is not this one's");
            assert_no_guard_held();
        });
    });
    assert_eq!(guards_held(), 1);
}

/// Both dedicated pools run their parallel half off the caller, and their
/// sequential half on any caller that is a rayon worker — whatever pool it
/// belongs to.
#[test]
fn a_dedicated_pool_runs_sequential_work_on_any_rayon_worker() {
    use crate::pools::{on_graph_pool, on_guarded_pool};

    let worker_ran = || crate::pools::on_a_rayon_worker();
    assert!(on_guarded_pool(worker_ran, || false));
    assert!(on_graph_pool(worker_ran, || false));

    for pool in [one_thread_pool(), crate::pools::application_pool(1)] {
        let parallel = pool.install(|| on_guarded_pool(|| true, || false));
        assert!(!parallel, "a rayon worker waited on the guarded pool");
        let parallel = pool.install(|| on_graph_pool(|| true, || false));
        assert!(!parallel, "a rayon worker waited on the graph pool");
    }
}

/// A dedicated pool that cannot be built answers `None`, and `run_on` runs
/// the sequential form of the work on the calling thread: the work still
/// makes progress — slower, and never deadlocked by a wait it cannot make.
#[test]
fn a_dedicated_pool_that_cannot_be_built_runs_sequential_work() {
    let pool = one_thread_pool();
    assert_eq!(
        crate::pools::run_on(Some(&pool), || "parallel", || "sequential"),
        "parallel",
        "a pool that built runs the parallel form"
    );
    assert_eq!(
        crate::pools::run_on(None, || "parallel", || "sequential"),
        "sequential",
        "no pool runs the sequential form, on this thread"
    );
}

#[test]
fn the_guarded_pool_lets_its_caller_hold_a_guard() {
    let lock = RwLock::new((0..1000_u64).collect::<Vec<_>>());
    let values = lock.read();
    let sums = crate::pools::on_guarded_pool(
        || crate::pools::map_chunks(&values, 100, |c| c.iter().sum::<u64>()),
        || values.chunks(100).map(|c| c.iter().sum::<u64>()).collect(),
    );
    drop(values);
    assert_eq!(sums.iter().sum::<u64>(), 499_500);
}

// ---------------------------------------------------------------------------
// No module but `pools` reaches a rayon pool
// ---------------------------------------------------------------------------

/// `source` with its comments taken out and its string and character
/// literals emptied, so only code is left to read. Line breaks are kept.
fn code_only(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        i = if c == '/' && next == Some('/') {
            skip_while(&chars, i, |c| c != '\n')
        } else if c == '/' && next == Some('*') {
            skip_block_comment(&chars, i, &mut out)
        } else if c == 'r' && matches!(next, Some('"' | '#')) && !prev_is_ident(&chars, i) {
            skip_raw_string(&chars, i, &mut out)
        } else if c == '"' {
            skip_quoted(&chars, i, '"', &mut out)
        } else if c == '\'' && is_char_literal(&chars, i) {
            skip_quoted(&chars, i, '\'', &mut out)
        } else {
            out.push(c);
            i + 1
        };
    }
    out
}

fn skip_while(chars: &[char], mut i: usize, keep: impl Fn(char) -> bool) -> usize {
    while i < chars.len() && keep(chars[i]) {
        i += 1;
    }
    i
}

/// Past the (possibly nested) block comment opening at `i`.
fn skip_block_comment(chars: &[char], mut i: usize, out: &mut String) -> usize {
    let mut depth = 0;
    while i < chars.len() {
        if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
            depth += 1;
            i += 2;
        } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
            depth -= 1;
            i += 2;
            if depth == 0 {
                break;
            }
        } else {
            if chars[i] == '\n' {
                out.push('\n');
            }
            i += 1;
        }
    }
    i
}

/// Past the raw string opening at `i` (an `r` not starting one is kept).
fn skip_raw_string(chars: &[char], i: usize, out: &mut String) -> usize {
    let mut j = skip_while(chars, i + 1, |c| c == '#');
    let hashes = j - i - 1;
    if chars.get(j) != Some(&'"') {
        out.push('r');
        return i + 1;
    }
    j += 1;
    out.push_str("\"\"");
    while j < chars.len() {
        if chars[j] == '"' && (0..hashes).all(|h| chars.get(j + 1 + h) == Some(&'#')) {
            return j + 1 + hashes;
        }
        if chars[j] == '\n' {
            out.push('\n');
        }
        j += 1;
    }
    j
}

/// Past the string or character literal `quote` opens at `i`.
fn skip_quoted(chars: &[char], mut i: usize, quote: char, out: &mut String) -> usize {
    out.push(quote);
    out.push(quote);
    i += 1;
    while i < chars.len() && chars[i] != quote {
        if chars[i] == '\\' {
            i += 1;
        } else if chars[i] == '\n' {
            out.push('\n');
        }
        i += 1;
    }
    i + 1
}

fn prev_is_ident(chars: &[char], i: usize) -> bool {
    i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_')
}

/// Whether the `'` at `i` opens a character literal rather than a lifetime.
fn is_char_literal(chars: &[char], i: usize) -> bool {
    match chars.get(i + 1) {
        Some('\\') => true,
        Some(_) => chars.get(i + 2) == Some(&'\''),
        None => false,
    }
}

/// One identifier in code-only source: its text, whether it is called
/// (`name(`) and with an empty argument list (`name()`), whether it follows
/// a `.`, its 1-based line, and the code before it on the line.
struct Ident {
    name: String,
    called: bool,
    empty_call: bool,
    method: bool,
    line: usize,
    before: String,
}

/// The identifiers of `code` (comments and literals already stripped).
fn idents(code: &str) -> Vec<Ident> {
    let mut found = Vec::new();
    for (line_no, line) in code.lines().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if !(chars[i].is_alphabetic() || chars[i] == '_') || prev_is_ident(&chars, i) {
                i += 1;
                continue;
            }
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let rest = chars[i..].iter().collect::<String>();
            let trimmed = rest.trim_start();
            found.push(Ident {
                name: chars[start..i].iter().collect(),
                called: trimmed.starts_with('('),
                empty_call: trimmed.starts_with("()"),
                method: start > 0
                    && chars[..start].iter().rev().find(|c| !c.is_whitespace()) == Some(&'.'),
                line: line_no + 1,
                before: chars[..start].iter().collect(),
            });
        }
    }
    found
}

/// rayon's parallel-iterator traits, named for a UFCS call or an import.
const RAYON_TRAITS: &[&str] = &[
    "ParallelIterator",
    "IndexedParallelIterator",
    "IntoParallelIterator",
    "IntoParallelRefIterator",
    "IntoParallelRefMutIterator",
    "FromParallelIterator",
    "ParallelSlice",
    "ParallelSliceMut",
    "ParallelExtend",
    "ParallelBridge",
    "ParallelString",
    "ParallelDrainRange",
    "ParallelDrainFull",
];

/// The ways a module can reach a rayon pool, each refused outside `pools`.
fn pool_violations(source: &str) -> Vec<String> {
    let code = code_only(source);
    let mut found = Vec::new();
    for ident in idents(&code) {
        let why = if ident.name == "rayon" {
            Some("names rayon")
        } else if ident.name.starts_with("ThreadPool") {
            Some("names a rayon thread pool")
        } else if RAYON_TRAITS.contains(&ident.name.as_str()) {
            Some("names a rayon parallel trait")
        } else if (ident.name.starts_with("par_") || ident.name.starts_with("into_par_"))
            && ident.called
        {
            Some("calls a parallel iterator")
        } else if ident.method
            && ident.called
            && matches!(
                ident.name.as_str(),
                "install" | "broadcast" | "spawn_broadcast"
            )
        {
            Some("submits to a thread pool")
        } else {
            None
        };
        if let Some(why) = why {
            found.push(format!("line {}: `{}` {why}", ident.line, ident.name));
        }
    }
    found
}

/// The marker a `std::thread` fan-out site must carry — in a `//` comment on
/// its line or in the comment block directly above it — saying why the
/// thread (or the wait on it) can take no collection lock. The colon is the
/// marker: ordinary prose like "takes no collection lock." must not excuse
/// a site, and neither may a `//` inside a same-line string literal that the
/// raw scan cannot tell from a comment — both read as unmarked.
const NO_LOCK_MARKER: &str = "no collection lock:";

/// The ways a module can wait on OS-thread fan-out — the deadlock of
/// `no_module_but_pools_names_rayon` one level down, needing no pool: a
/// thread holding a collection guard that joins a thread taking one
/// deadlocks identically. Every site carries [`NO_LOCK_MARKER`] saying why
/// it cannot close that cycle. `thread::spawn` is refused on sight: the scan
/// cannot tell a handle that will be dropped from one that will be joined,
/// so the marker writes down which it is.
fn fanout_violations(source: &str) -> Vec<String> {
    let code = code_only(source);
    let raw: Vec<&str> = source.lines().collect();
    let mut found = Vec::new();
    for ident in idents(&code) {
        let why = if ident.called
            && !ident.method
            && matches!(ident.name.as_str(), "spawn" | "scope")
            && ident.before.trim_end().ends_with("thread::")
        {
            Some("spawns or joins threads")
        } else if ident.name == "join"
            && ((ident.method && ident.empty_call)
                || (ident.called
                    && !ident.method
                    && ident.before.trim_end().ends_with("JoinHandle::")))
        {
            Some("joins a thread")
        } else {
            None
        };
        if let Some(why) = why {
            if !marked(&raw, ident.line) {
                found.push(format!("line {}: `{}` {why}", ident.line, ident.name));
            }
        }
    }
    found
}

/// Whether the call flagged on `lines[line - 1]` carries [`NO_LOCK_MARKER`]:
/// on the call's line, in the comment block directly above it, or — when the
/// call continues a statement, like a `.join()` rustfmt split into a chain —
/// in the comment block directly above the statement's first line.
fn marked(lines: &[&str], line: usize) -> bool {
    let mut statement = line - 1;
    while statement > 0 && lines[statement].trim_start().starts_with('.') {
        statement -= 1;
    }
    marked_at(lines, line - 1) || marked_at(lines, statement)
}

/// [`NO_LOCK_MARKER`] on `lines[i]` or in the comment block directly above.
fn marked_at(lines: &[&str], i: usize) -> bool {
    if let Some(comment) = lines[i].find("//") {
        if lines[i][comment..].contains(NO_LOCK_MARKER) {
            return true;
        }
    }
    let mut j = i;
    while j > 0 {
        j -= 1;
        let above = lines[j].trim_start();
        if !above.starts_with("//") {
            return false;
        }
        if above.contains(NO_LOCK_MARKER) {
            return true;
        }
    }
    false
}

/// Every way around `pools` the review found, refused.
#[test]
fn the_pool_scan_refuses_each_way_around_pools() {
    let bypasses = [
        ("a bare join imported from rayon", "use rayon::join;\nfn f() { join(|| 1, || 2); }"),
        ("join_context", "fn f() { rayon::join_context(|_| 1, |_| 2); }"),
        ("broadcast", "fn f(p: &P) { p.broadcast(|_| 1); }"),
        ("par_rchunks", "fn f(v: &[u8]) { v.par_rchunks(2).count(); }"),
        ("UFCS par_iter", "fn f(v: &[u8]) { IntoParallelRefIterator::par_iter(v); }"),
        ("install", "fn f(p: &P) { p.install(|| 1); }"),
        (
            "an assert before the guard",
            "fn f(l: &L) { crate::lock_audit::assert_no_guard_held(); let g = l.read(); g.par_iter(); }",
        ),
        (
            "a par_iter after an unrelated guarded run",
            "fn f(v: &[u8]) { pools::on_guarded_pool(|| 1, || 1); v.par_iter().count(); }",
        ),
        ("a thread pool", "fn f() -> Option<ThreadPool> { None }"),
        ("a glob import", "use rayon::prelude::*;"),
        ("an alias", "extern crate rayon as r;"),
    ];
    for (name, source) in bypasses {
        assert!(
            !pool_violations(source).is_empty(),
            "the scan missed {name}"
        );
    }
}

/// What is not a submission stays allowed: helpers of `pools`, the word in
/// comments and strings, identifiers that only contain it.
#[test]
fn the_pool_scan_reads_code_only() {
    let allowed = r##"
        // rayon::join in a comment, ThreadPool too, and v.par_iter()
        /* block: p.install(|| 1) */
        /// doc: rayon
        fn brute_force_search_rayon() -> &'static str { "rayon::join, .install(" }
        fn f(v: &[u8]) -> Vec<u8> { crate::pools::map(v, |x| *x) }
        fn g() -> &'static str { r#"ThreadPool "par_iter("#; 'x'; "a\"rayon" }
        fn parse(x: u8) -> u8 { x }
        struct ParallelConfig; enum S { Parallel }
    "##;
    assert_eq!(pool_violations(allowed), Vec::<String>::new());
}

fn rust_sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("test: read src") {
        let path = entry.expect("test: dir entry").path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `(path, text)` of each non-test `.rs` source of this crate. Test files
/// are exempt from both scans: they stage pools and threads.
fn prod_sources() -> Vec<(std::path::PathBuf, String)> {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_sources(&src, &mut files);
    files
        .into_iter()
        .filter(|path| {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            !(name.ends_with("_tests.rs") || name == "tests.rs")
        })
        .map(|path| {
            let text = std::fs::read_to_string(&path).expect("test: read source");
            (path, text)
        })
        .collect()
}

/// Every wait on spawned threads the review found, refused without the
/// marker.
#[test]
fn the_fanout_scan_refuses_each_unjustified_wait() {
    let waits = [
        (
            "a scoped thread",
            "fn f() { std::thread::scope(|s| { s.spawn(|| 1); }); }",
        ),
        (
            "a spawned handle joined later",
            "fn f() { let h = std::thread::spawn(|| 1); h.join(); }",
        ),
        (
            "spawn().join()",
            "fn f() { std::thread::spawn(|| 1).join(); }",
        ),
        (
            "a UFCS join",
            "fn f(h: std::thread::JoinHandle<()>) { std::thread::JoinHandle::join(h); }",
        ),
        (
            "a spawn the scan cannot prove detached",
            "fn f() { std::thread::spawn(|| 1); }",
        ),
        (
            "prose naming the marker without its colon",
            "fn f() {\n    // takes no collection lock. The merge scores pending points under the\n    std::thread::scope(|s| { s.spawn(|| 1); });\n}",
        ),
        (
            "a `//` inside a same-line string literal",
            "fn f() { let s = \"docs say: // no collection lock is the marker\"; std::thread::spawn(|| 1); }",
        ),
        (
            "a join a marker on another line does not cover",
            "fn f() { let h = std::thread::spawn(|| 1); }\nfn g() { h.join(); }",
        ),
    ];
    for (name, source) in waits {
        assert!(
            !fanout_violations(source).is_empty(),
            "the scan missed {name}"
        );
    }
}

/// What is not a wait on thread fan-out stays allowed: the marker on the
/// line or the comment block above it, a `join` that takes an argument, a
/// `spawn` that is not `std::thread`'s, and the words in comments.
#[test]
fn the_fanout_scan_allows_marked_and_non_wait_forms() {
    let allowed = r#"
        // no collection lock: the thread only probes the GPU.
        std::thread::spawn(init_device).join().ok().flatten()
        // no collection lock: a marker above the statement covers a mid-chain join.
        std::thread::spawn(init_device)
            .join()
            .ok()
            .flatten()
        // no collection lock: the handle is dropped — nothing can wait on it.
        std::thread::spawn(move || { probe_update(); });
        // no collection lock: `work` only reads the config file it was given.
        let h = std::thread::spawn(work); h.join()
        std::thread::spawn(work); // no collection lock: a marker on the call's own line.
        fn f(v: &[&str]) -> String { v.join(", ") }
        fn g(h: H) { h.join(other) }
        fn t() { tokio::spawn(probe()); }
        // std::thread::spawn in a comment, and "std::thread::scope" in a string.
        fn s() -> &'static str { "std::thread::scope" }
    "#;
    assert_eq!(fanout_violations(allowed), Vec::<String>::new());
}

/// The same rule one lock down: no module waits on `std::thread` fan-out —
/// `thread::spawn`, `thread::scope`, a `JoinHandle`'s `join` — unless the
/// site carries `no collection lock:`, the marker naming why it cannot close
/// a cycle.
#[test]
fn no_unjustified_thread_fanout() {
    let mut violations = Vec::new();
    let mut scanned = 0;
    for (path, text) in prod_sources() {
        scanned += 1;
        for violation in fanout_violations(&text) {
            violations.push(format!("{}: {violation}", path.display()));
        }
    }
    assert!(scanned > 100, "the scan read {scanned} files");
    assert!(
        violations.is_empty(),
        "waiting on spawned threads under a collection guard deadlocks like \
         rayon does: mark the site `no collection lock: <why it cannot>`:\n{}",
        violations.join("\n")
    );
}

/// No module of the crate but `pools` names rayon, a thread pool or a
/// parallel iterator, so every submission goes through a helper of `pools`,
/// which holds the rule for it. Test files are exempt: they stage pools.
#[test]
fn no_module_but_pools_names_rayon() {
    let mut violations = Vec::new();
    let mut scanned = 0;
    for (path, text) in prod_sources() {
        if path.file_name().and_then(|n| n.to_str()) == Some("pools.rs") {
            continue;
        }
        scanned += 1;
        for violation in pool_violations(&text) {
            violations.push(format!("{}: {violation}", path.display()));
        }
    }
    assert!(scanned > 100, "the scan read {scanned} files");
    assert!(
        violations.is_empty(),
        "reach rayon through crate::pools only:\n{}",
        violations.join("\n")
    );
}
