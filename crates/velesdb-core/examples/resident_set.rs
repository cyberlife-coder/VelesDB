//! Measures what a quantized index actually costs in un-evictable RAM.
//!
//! #2112 asks for the resident set of a 100 000 × 768-d SQ8 index to be
//! `codes + graph`, "demonstrated in a test or bench artifact — not asserted
//! in prose", plus the cold-page re-rank cost that evictability buys. This is
//! that artifact.
//!
//! # Why `VmRSS` is the wrong number
//!
//! A mapped file's pages count toward `VmRSS` for as long as they are
//! resident, so an index whose f32 arena is file-backed reports the *same*
//! `VmRSS` as one whose arena is on the heap — right up until memory pressure
//! arrives. Reading `VmRSS` and concluding "no saving" would be wrong, and
//! waiting for pressure would measure the host, not the change.
//!
//! What changed is which *kind* of page holds the f32, and Linux reports that
//! directly in `/proc/self/status`:
//!
//! - `RssAnon` — anonymous pages, reclaimable only by swapping. The
//!   edge/IoT device this feature targets usually has no swap, so this is the
//!   floor that decides whether the process fits at all.
//! - `RssFile` — file-backed pages, reclaimable by dropping them, because
//!   the file already holds their contents.
//!
//! The file-backed arena moves `N × D × 4` bytes from the first to the
//! second. That is the whole claim, and it is a difference this program reads
//! rather than infers.
//!
//! # One mode per process, always
//!
//! The first collection built in a process absorbs every one-time cost the
//! process pays — thread pools, allocator arenas, lazy statics. Measured back
//! to back in one run, `Full` charged +310 MiB of `RssAnon` for a 14.6 MiB
//! arena while `SQ8` charged +16 MiB, and almost all of that gap was the
//! order, not the mode. So this program measures **one** mode per invocation
//! and the comparison is made between runs.
//!
//! # The two halves are measured at different levels, on purpose
//!
//! **Resident set** goes through `Database`, the real production path: only a
//! collection owns the directory that makes its arena file-backed, and a
//! number measured anywhere else would not be the number a user pays.
//!
//! **Cold-page cost** goes through `ContiguousVectors` directly. Evicting a
//! live collection's arena would need a public API whose only caller is this
//! program, and the page-fault cost is a property of the arena, not of the
//! graph above it — isolating it removes traversal noise from a figure that
//! is meant to be about I/O. What a re-rank does to the arena is read `k`
//! scattered vectors, which is exactly what the cold loop below times.
//!
//! # Two kinds of cold
//!
//! `MADV_DONTNEED` drops the *process's* pages, but the file's contents can
//! still sit in the page cache, so the next touch is a minor fault — cheap,
//! and not what evictability costs. Reclaim under real pressure takes the page
//! cache too, and the next touch is a disk read. Both are reported: the minor
//! fault is the floor, the post-`POSIX_FADV_DONTNEED` figure is what an
//! edge device without swap actually pays.
//!
//! # Usage
//!
//! ```text
//! cargo run --release --example resident_set --features persistence -- <mode> [N] [D]
//! ```
//!
//! `mode` is `full`, `sq8`, or `cold`, and only one runs per invocation — see
//! above for why. `N` and `D` default to the configuration #2112 names, so the
//! three runs behind the published tables are `-- full`, `-- sq8` and
//! `-- cold`. Linux only: no other platform
//! exposes the anonymous/file split, and reporting `VmRSS` alone would be
//! reporting the very number this program exists to reject.

fn main() {
    #[cfg(all(target_os = "linux", feature = "persistence"))]
    linux::run();
    #[cfg(not(all(target_os = "linux", feature = "persistence")))]
    eprintln!(
        "resident_set needs Linux (for the RssAnon/RssFile split) and \
         --features persistence (for the file-backed arena)."
    );
}

#[cfg(all(target_os = "linux", feature = "persistence"))]
mod linux {
    use std::time::{Duration, Instant};
    use velesdb_core::perf_optimizations::ContiguousVectors;
    use velesdb_core::{Database, DistanceMetric, Point, StorageMode};

    /// The split that decides whether an index fits on a small device.
    #[derive(Clone, Copy)]
    struct Rss {
        anon_kb: u64,
        file_kb: u64,
    }

    impl Rss {
        fn read() -> Self {
            let status = std::fs::read_to_string("/proc/self/status")
                .expect("Linux always exposes /proc/self/status");
            Self {
                anon_kb: field(&status, "RssAnon:"),
                file_kb: field(&status, "RssFile:"),
            }
        }
    }

    /// Pulls one `kB` field out of `/proc/self/status`.
    fn field(status: &str, key: &str) -> u64 {
        status
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .and_then(|v| v.split_whitespace().next())
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| panic!("{key} missing from /proc/self/status"))
    }

    /// kB, as `/proc` reports it, to MiB.
    ///
    /// The cast is lossy above 2^53 kB — eight petabytes of resident memory,
    /// which no reading here will reach.
    #[allow(clippy::cast_precision_loss)]
    fn mib(kb: u64) -> f64 {
        kb as f64 / 1024.0
    }

    /// Deterministic vectors: reproducible runs, and no RNG in the measurement.
    ///
    /// The cast is exact: the modulo bounds the value below 1000, well inside
    /// an f32 mantissa.
    #[allow(clippy::cast_precision_loss)]
    fn vector(seed: usize, dimension: usize) -> Vec<f32> {
        (0..dimension)
            .map(|d| ((seed * 31 + d * 17) % 1000) as f32 / 1000.0 - 0.5)
            .collect()
    }

    /// Builds one collection and reports what it added to each kind of RSS.
    ///
    /// Hands the database back so the caller can hold it: dropping it would
    /// release the arena and make the reading describe memory nobody holds.
    fn build(
        label: &str,
        mode: StorageMode,
        count: usize,
        dimension: usize,
    ) -> (Database, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("a writable temp dir");
        let before = Rss::read();
        let db = Database::open(dir.path()).expect("database opens");
        db.create_vector_collection_with_options("bench", dimension, DistanceMetric::Cosine, mode)
            .expect("collection creates");
        let coll = db
            .get_vector_collection("bench")
            .expect("the collection just created");

        let started = Instant::now();
        for chunk in (0..count).step_by(1_000) {
            let points: Vec<Point> = (chunk..(chunk + 1_000).min(count))
                .map(|i| Point::without_payload(i as u64, vector(i, dimension)))
                .collect();
            coll.upsert(points).expect("upsert succeeds");
        }
        let after = Rss::read();

        println!(
            "{label:<28} {:>6.1}s   RssAnon +{:>8.1} MiB   RssFile +{:>8.1} MiB",
            started.elapsed().as_secs_f64(),
            mib(after.anon_kb.saturating_sub(before.anon_kb)),
            mib(after.file_kb.saturating_sub(before.file_kb)),
        );
        (db, dir)
    }

    /// Median — robust to the one slow sample a shared runner always produces.
    fn median(mut samples: Vec<Duration>) -> Duration {
        samples.sort_unstable();
        samples[samples.len() / 2]
    }

    /// Times `rounds` re-rank-shaped reads: `k` scattered vectors each.
    ///
    /// Every element is summed, not just the first. A vector is `D × 4` bytes
    /// — 3 KiB at 768 dims — so it straddles one or two pages, and a re-rank
    /// computing a distance touches all of them. Reading only `v[0]` would
    /// fault a single page per vector and undercount the very thing this
    /// measures. `black_box` keeps the sum from being optimised away.
    fn time_scattered_reads(
        arena: &ContiguousVectors,
        count: usize,
        k: usize,
        rounds: usize,
    ) -> Duration {
        let samples: Vec<Duration> = (0..rounds)
            .map(|r| {
                let started = Instant::now();
                for i in 0..k {
                    // A stride coprime with `count` visits a different scatter
                    // each round without repeating within one.
                    let idx = (r * 7919 + i * 104_729) % count;
                    let v = arena.get(idx).expect("index in range");
                    std::hint::black_box(v.iter().sum::<f32>());
                }
                started.elapsed()
            })
            .collect();
        median(samples)
    }

    /// The cast is lossy only past 2^53 bytes of arena, which would need more
    /// vectors than the address space holds.
    #[allow(clippy::cast_precision_loss)]
    pub fn run() {
        let args: Vec<String> = std::env::args().collect();
        let mode_arg = args.get(1).map_or("sq8", String::as_str);
        let count: usize = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(100_000);
        let dimension: usize = args.get(3).and_then(|a| a.parse().ok()).unwrap_or(768);
        let f32_mib = (count * dimension * 4) as f64 / (1024.0 * 1024.0);

        let (mode, label) = match mode_arg {
            "full" => (StorageMode::Full, "Full (heap arena)"),
            "sq8" => (StorageMode::SQ8, "SQ8 (file-backed arena)"),
            "cold" => {
                cold_page_cost(count, dimension);
                return;
            }
            other => {
                eprintln!("unknown mode {other:?}; expected full | sq8 | cold");
                std::process::exit(2);
            }
        };

        println!("Resident set — {count} × {dimension}-d");
        println!(
            "f32 arena {f32_mib:.1} MiB · SQ8 codes {:.1} MiB\n",
            f32_mib / 4.0
        );

        let (db, _dir) = build(label, mode, count, dimension);
        // Alive to here: the reading above describes memory that must still be
        // held for it to mean anything.
        drop(db);
    }

    /// The honest price of evictability, isolated from graph traversal.
    fn cold_page_cost(count: usize, dimension: usize) {
        const K: usize = 100;
        const ROUNDS: usize = 50;

        let dir = tempfile::tempdir().expect("a writable temp dir");
        let path = dir.path().join("arena.bin");
        let mut arena = ContiguousVectors::new_file_backed(&path, dimension, count)
            .expect("a file-backed arena");
        for i in 0..count {
            arena.push(&vector(i, dimension)).expect("push succeeds");
        }

        println!("Cold-page re-rank cost — {K} scattered vectors, median of {ROUNDS} rounds");
        let warm = time_scattered_reads(&arena, count, K, ROUNDS);
        report("warm", warm, warm);

        // Minor fault: the process loses its pages, the page cache keeps them.
        let before = Rss::read();
        arena.evict_backing().expect("eviction succeeds");
        let after = Rss::read();
        println!(
            "  dropped{:>9.1} MiB of RssFile",
            mib(before.file_kb.saturating_sub(after.file_kb)),
        );
        report(
            "minor",
            time_scattered_reads(&arena, count, K, ROUNDS),
            warm,
        );

        // Major fault: take the page cache too, which is what reclaim under
        // real memory pressure does. The arena was flushed by `evict_backing`,
        // so these pages are clean and the kernel can drop them.
        arena.evict_backing().expect("eviction succeeds");
        drop_page_cache(&path);
        report(
            "major",
            time_scattered_reads(&arena, count, K, ROUNDS),
            warm,
        );
    }

    /// Prints one latency line against the warm baseline.
    fn report(label: &str, measured: Duration, warm: Duration) {
        let ms = measured.as_secs_f64() * 1e3;
        if label == "warm" {
            println!("  {label:<7}{ms:>9.3} ms");
        } else {
            println!(
                "  {label:<7}{ms:>9.3} ms   ({:.1}× warm, +{:.3} ms)",
                measured.as_secs_f64() / warm.as_secs_f64().max(f64::MIN_POSITIVE),
                ms - warm.as_secs_f64() * 1e3,
            );
        }
    }

    /// Evicts a file's clean pages from the page cache.
    ///
    /// Best-effort and deliberately quiet: this only sharpens the measurement,
    /// and a kernel that declines leaves the `major` row equal to `minor`,
    /// which is visible in the output rather than hidden.
    fn drop_page_cache(path: &std::path::Path) {
        use std::os::unix::io::AsRawFd;
        let Ok(file) = std::fs::File::open(path) else {
            return;
        };
        // SAFETY: `posix_fadvise` over a file descriptor this function owns
        // and keeps alive for the call.
        // - Condition 1: `file` is open and its fd valid until it drops below.
        // - Condition 2: offset 0 / len 0 means "the whole file"; `DONTNEED`
        //   only drops clean page-cache pages, so no data can be lost.
        // SAFETY: Take the page cache too, so the next touch is a real read.
        unsafe {
            libc::posix_fadvise(file.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED);
        }
    }
}
