# Migrating a store to a new embedding model

`velesdb-memory migrate-embeddings` rebuilds an existing store against a new
embedding model — new vectors, same facts, same ids, same metadata, same
absolute expiries, same graph edges with their properties — and switches the
rebuilt store into place. It exists because vectors from two different models
are not comparable: pointing the daemon at a store built by another model makes
recall silently return nonsense, which is why the daemon refuses to start on a
model mismatch and points you at this command.

Everything below is journalled, resumable, and refuses loudly rather than
guessing. The one command is:

```bash
velesdb-memory migrate-embeddings --store <dir> --destination <dir> [--strategy auto|reuse|reembed]
```

## Before you start

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
