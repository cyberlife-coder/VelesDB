//! Tests for the collection lock audit: each half of the rule it checks
//! panics when broken, and every rayon submission in the crate is checked.

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

#[test]
fn a_guard_held_across_a_global_pool_submission_panics() {
    let lock = RwLock::new(0_u32);
    let mutex = Mutex::new(0_u32);

    let read = lock.read();
    assert_eq!(guards_held(), 1);
    assert!(
        catch_unwind(assert_no_guard_held).is_err(),
        "a submission was let through with a read guard held"
    );
    drop(read);
    let locked = mutex.lock();
    assert!(
        catch_unwind(assert_no_guard_held).is_err(),
        "a submission was let through with a mutex guard held"
    );
    drop(locked);
    assert_eq!(guards_held(), 0, "a dropped guard is no longer counted");
    assert_no_guard_held();
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

#[test]
fn the_guarded_pool_runs_parallel_work_off_the_caller_and_sequential_work_on_a_worker() {
    use crate::collection::guarded_pool::run;

    let (on_pool, ran_parallel) = run(
        || (rayon::current_thread_index().is_some(), true),
        || (false, false),
    );
    assert!(
        ran_parallel && on_pool,
        "the parallel half runs on the pool"
    );

    let ran_parallel = one_thread_pool().install(|| run(|| true, || false));
    assert!(
        !ran_parallel,
        "a rayon worker runs the sequential half itself"
    );
}

#[test]
fn the_guarded_pool_lets_its_caller_hold_a_guard() {
    use rayon::prelude::*;

    let lock = RwLock::new((0..1000_u64).collect::<Vec<_>>());
    let values = lock.read();
    let sum: u64 =
        crate::collection::guarded_pool::run(|| values.par_iter().sum(), || values.iter().sum());
    drop(values);
    assert_eq!(sum, 499_500);
}

/// A rayon submission: work handed to a pool, which the caller waits on.
/// The same alternation as `scripts/check_hnsw_rayon_pool.py`.
const SUBMISSIONS: &[&str] = &[
    ".par_iter(",
    ".par_iter_mut(",
    ".par_chunks(",
    ".par_chunks_mut(",
    ".par_chunks_exact(",
    ".par_chunks_exact_mut(",
    ".par_bridge(",
    ".into_par_iter(",
    ".par_drain(",
    ".par_extend(",
    ".par_split(",
    ".par_split_mut(",
    ".par_windows(",
    ".par_sort",
    "rayon::join(",
    "rayon::scope(",
    "rayon::spawn(",
    "rayon::in_place_scope(",
    "rayon::scope_fifo(",
    "rayon::spawn_fifo(",
];

/// What makes a submission checked: the held-guard assert, or the pool for
/// work over guarded data, earlier in the same function.
const CHECKS: &[&str] = &["lock_audit::assert_no_guard_held()", "guarded_pool::run("];

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

fn is_fn_start(line: &str) -> bool {
    let code = line.trim_start();
    !code.starts_with("//")
        && (code.starts_with("fn ")
            || code.contains(" fn ")
            || code.starts_with("pub fn ")
            || code.starts_with("pub(crate) fn ")
            || code.starts_with("pub(super) fn "))
}

/// Every rayon submission outside the tests is checked by the audit, so a
/// unit test reaching it with a collection guard held fails.
///
/// "Checked" is lexical: the assert, or `guarded_pool::run(`, appears between
/// the submission and the start of the function holding it. That is what a
/// new submission's author has to write; the unit tests then hold it.
#[test]
fn every_rayon_submission_in_the_crate_is_checked() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_sources(&src, &mut files);
    let mut unchecked = Vec::new();
    let mut submissions = 0;
    for path in files {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.ends_with("_tests.rs") || name == "lock_audit.rs" {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("test: read source");
        let lines: Vec<&str> = text.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if !SUBMISSIONS.iter().any(|s| code.contains(s)) {
                continue;
            }
            submissions += 1;
            let checked = lines[..=index]
                .iter()
                .rev()
                .take_while(|l| !is_fn_start(l))
                .chain(lines[..=index].iter().rev().find(|l| is_fn_start(l)))
                .any(|l| CHECKS.iter().any(|c| l.contains(c)));
            if !checked {
                unchecked.push(format!("{}:{}", path.display(), index + 1));
            }
        }
    }
    assert!(submissions > 0, "the scan found no submission at all");
    assert!(
        unchecked.is_empty(),
        "rayon submissions without `crate::lock_audit::assert_no_guard_held()` or \
         `collection::guarded_pool::run` before them in their function:\n{}",
        unchecked.join("\n")
    );
}
