# Migrating a store to a new embedding model

`velesdb-memory migrate-embeddings` rebuilds an existing store against a new
embedding model — new vectors, same facts, same ids, same metadata, same
absolute expiries, same graph edges with their properties — and switches the
rebuilt store into place. It exists because vectors from two different models
are not comparable: pointing the daemon at a store built by another model makes
recall silently return nonsense, which is why the daemon refuses to start on a
model mismatch and points you at this command.

Everything below is journalled, resumable, and refuses loudly rather than
guessing. For a stopped daemon, the command is:

```bash
velesdb-memory migrate-embeddings --store <dir> --destination <dir> [--strategy auto|reuse|reembed]
```

## Choose online or offline

Use the daemon-owned online path when the daemon must keep serving. Use the
offline command when an outage is acceptable or when you want to stage an
explicit destination yourself. Never run the offline command against a store
the daemon has open; its process lock refuses that unsafe combination.

## Online migration through MCP

The online control plane uses the daemon's existing MCP transport and its
existing authorization boundary. It adds no listener. The target backend is
resolved from the daemon environment; credentials and endpoint secrets are
never copied into durable job state.

1. Submit this `migration_start` request with the backend and the maximum
   request pause you accept:

   ```json
   {
     "target_backend": "ollama",
     "pause_budget_ms": 500,
     "journal_max_bytes": 67108864,
     "fact_batch": 256,
     "replay_batch": 256,
     "edge_cap": 4096,
     "observation_window": 3,
     "verification_reserve_ms": 100
   }
   ```

   `migration_start` returns `{ configured, job }` immediately after the job
   is durable. The work continues in the background.

2. Poll `migration_status`. Progress reports base-copy counts, input/output
   watermarks, dirty fact and edge-source counts, pending journal bytes, the
   estimated pause, the measured cutover, the last error, and any mandatory
   recovery action.
3. A `non_converging` job has stopped rather than looped forever. Reduce the
   write rate or increase replay capacity, then call `migration_recover`; the
   daemon revalidates the target model, dimension and vector witness before it
   resumes.
4. `migration_cancel` is accepted only before quiescing while the source is
   authoritative. It verifies the epoch journal and target provenance before
   removing the generated destination and journal. From `quiescing` onward it
   refuses and reports the recovery action instead of guessing a rollback.
5. After a daemon crash, restart it with the same store and target-backend
   configuration. Startup repairs an interrupted cutover before opening the
   store: pre-activation state rolls back to the source; post-activation state
   completes forward to the target. For an earlier stopped phase, call
   `migration_recover` after restart.

The destination is `<store>.online-migration-target`, the bounded dirty-state
journal is `<store>.online-migration-target.migration-journal`, and durable job
status is under `<store>.online-migration-control`. Pre-existing paths,
symlinks, corrupt/future state, identity mismatches and an in-place target-model
change are refusals. The journal byte cap is a safety boundary: when full, a
source write is refused before it could become untracked.

## Before an offline run

- **Stop the daemon.** The store is single-writer; a live daemon holds its
  `flock` and the migration refuses to open the source. Do not race it: a
  daemon started mid-migration can only lose.
- **Pick a destination on the same filesystem** as the store — the final
  switch is a filesystem rename, and a rename cannot cross volumes. An empty
  or not-yet-existing directory; the migration refuses one that already holds
  data no journal accounts for.
- **Have room.** The diagnosis stages a verified copy of the store next to it,
  and the rebuild writes a full second store.

## Step 1 — dry-run

```bash
velesdb-memory migrate-embeddings --store <dir> --dry-run
```

Read the first line first: it names the **regime** this store resolves to.
Then the blockers. Two of them (`disk_headroom`, `embedder_cost`) are
permanently informational — they name measurements only you can take on your
machine — and do not stop the run. The regime line does: a `REFUSE` regime
exits 2, and its own text says which way out it leaves you. A refused *reuse*
has no override — no flag turns it into a run, by design — while a store whose
own records contradict each other can still be re-embedded explicitly
(`--strategy reembed`), which reads nothing the contradiction touches.

## The regimes

- **`auto`** (default): re-embeds, unless reuse is *proven* safe — with one
  refusal of its own: a store whose provenance record contradicts its measured
  dimension is not decided on your behalf. `auto` refuses it and leaves the
  explicit choice (`--strategy reembed`) to you.
- **`reembed`**: every fact's `content` goes through the target embedder.
- **`reuse`**: the source vectors are copied verbatim. This is allowed only
  when the store's recorded provenance names the SAME model at the SAME
  dimension as the target — anything less is refused, never downgraded to a
  guess. **Reuse is not an embedding migration**: it never calls an embedder
  (guaranteed structurally — the reuse path has no embedder to call, which a
  test pins by running a whole migration with an embedder that panics on
  use). It is a store rebuild that happens to keep the vectors, useful after
  corruption or for compaction, and its cost is the reinsertion cost alone.

There is deliberately no `--force-reuse`. A flag that overrides the proof
would produce a store whose vectors and recorded model disagree — the exact
condition this command exists to prevent.

## Step 2 — the run

A non-dry-run performs, in order, journalling each step durably:

1. **Rebuild** into the destination, collection by collection, checkpointed
   after every batch. Kill it anywhere; the re-run replays at most one batch,
   visibly (reported as "already present"), and loses nothing.
2. **Validation**: one pass comparing every fact (ids and payloads) and every
   edge (complete tuples, properties included) between source and
   destination. The destination is then stamped with the target model's
   provenance — the record the daemon checks at startup.
3. **The switch**: the source is renamed aside to `<store>.archive`, the
   destination is renamed to the store's path, the activated store is
   verified, and the archive is freed. Both renames are guarded by the
   journalled source fingerprint: a tree that changed hands since the journal
   was written refuses to move or be deleted, whoever changed it.

Then **restart the daemon**. It holds its store as an in-memory handle taken
at startup and never refreshed; until it restarts it keeps serving the old
data from memory. The completion message says this too, every time.

## If something stops

**Re-run the same command.** The journal — a `<destination>.migration-journal`
directory beside the destination path — records how far every stage got, and
the re-run enters the chain exactly there, including after the switch's first
rename (when the store's path is temporarily vacant).

Refusals you may meet, and what they mean:

- *"a migration lock record remains"* — a previous run crashed hard (a clean
  failure releases the lock). Inspect, then delete
  `<destination>.migration-journal/migration.lock` yourself; it is left as
  evidence deliberately and is never stolen by a liveness guess.
- *"the source changed since this migration was prepared"* — something wrote
  to the source after the rebuild started. Start a fresh diagnosis; resuming
  would rebuild from an inventory that no longer describes the store.
- *"Same name, different vectors"* — the model behind the
  target's name was updated in place (an `ollama pull` does this). The
  journal carries a witness of what the embedder actually produces; resuming
  across an update would mix two vector spaces in one store. Start fresh.
- *"does not carry the target's provenance stamp"* / *"no longer fingerprints
  as the store this journal describes"* — the disk does not match any state
  this migration produced. Nothing was moved or deleted; inspect by hand.

If you followed the recovery table's manual advice and moved the archive back
to the store's name: re-running the switch recognises the restored source
(fingerprint-checked), re-archives it, and completes.

## What is left behind

- The **journal** (`<destination>.migration-journal/`): the durable record of
  what happened, including the lock guard file. Keep it as long as you want
  the evidence; remove it when you no longer do.
- The **archive** (`<store>.archive`) exists only between the switch's first
  rename and the commit, which frees it — after verifying both that the
  activated store opens and carries the target's stamp, and that the archive
  still fingerprints as the source the journal knows. An archive that
  received foreign writes refuses to die.

## What it costs

No duration is promised, because none transfers between machines. What has
been measured (each figure on one machine, quoted with what it is):

### Online-path evidence

Measured 2026-08-15 on arm64 macOS 26.5.2, optimized build, offline hash
embedder at 384 dimensions, 64 seeded facts and 16 measured writes per arm:

```text
baseline_us_per_write=10231.3
capture_us_per_write=19214.4
capture_ratio=1.88
replayed_records=8
migration_ms=2141
```

This is a reproducible diagnostic, not an SLA. Run it on the deployment host:

```bash
cargo test -p velesdb-memory --release --test online_migration_process \
  reports_steady_state_capture_and_replay_overhead -- \
  --ignored --test-threads=1 --nocapture
```

The no-capture generation guard was also isolated with Criterion on the same
machine: direct `NativeStore::count` measured 5.45 ns and guarded
`MemoryService::fact_count` 6.18 ns, an absolute 0.73 ns increment. The 13%
relative number is intentionally the worst-looking view of a sub-10 ns
synthetic operation; against the measured 10.2 ms end-to-end write it is below
the resolution of the request-scale measurement. Re-run with:

```bash
cargo bench -p velesdb-memory --bench generation_gate_benchmark -- --noplot
```

Capture itself is deliberately not free: every acknowledged mutation first
appends and durability-syncs its dirty key. On this run write latency was
1.88× baseline. Size the pause and journal from measurements on the actual
filesystem and embedder, and treat `non_converging` as a capacity signal rather than
repeatedly forcing recovery.

### Offline-path evidence

- **Re-insertion** — the whole cost under `reuse` — measured at ~16 µs/fact
  (dimension 4, `--release`, batch 1024) after the batched-fsync fix (#1797);
  the cost per fact *decreases* with volume as fixed costs amortise. A
  million-fact rebuild is seconds-to-minutes territory, not hours.
- **Re-embedding** was measured at roughly **23×** a re-insertion (bge-m3 at
  1024 dimensions, one machine, one unbatched embedder call per fact, debug
  build — and the dominant denominator term turned out to be BM25 text
  indexing, not vector width). This is a ratio from one configuration, not a
  promise: it does not transfer to another embedder, backend, or batch size.
  Under `reembed`, the embedder is your budget; measure it with your model on
  your machine before quoting a duration to anyone.

The performance suite (`migration/tests/performance.rs`) carries the
deliberately-run benchmarks behind `#[ignore]`; run them on a machine at rest.
