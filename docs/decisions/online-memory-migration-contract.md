# Online memory migration is a daemon-owned dirty-state protocol

Status: accepted design; not implemented

Issue [#1796](https://github.com/cyberlife-coder/VelesDB/issues/1796)
requires an embedding migration that keeps the daemon available during the
long rebuild. This document is the global contract that issue requires before
implementation. It does not make online migration available, and no
intermediate implementation may claim that it does.

## Decision

The live daemon owns the complete online migration. It keeps serving the source
store while it incrementally builds the destination, captures every concurrent
mutation in a durable dirty-state journal, catches the destination up, and
admits a short final cutover only when an operator-supplied pause budget can be
honoured.

The protocol records *what must be re-read*, not what an operation intended to
do. A fact mutation dirties its fact id. A graph mutation dirties the source id
whose outgoing edge set changed. Replay reads the current authoritative state
from the source and makes the destination match it. Replaying the same record
is therefore safe, and a journal record left by a source operation that later
failed is only a harmless extra read.

The existing `migrate-embeddings` command remains the offline path until every
gate in this document is implemented. An external process never opens or
renames the live store: `Database::open` holds an exclusive process lock, and
the current switch code documents that renaming a directory below a live
handle can let the daemon write into an archive that is later unlinked.

## Terms

- **Epoch**: one immutable migration identity: source path and provenance,
  target model and dimension, destination path, and a random epoch id.
- **Sequence**: a monotonically increasing journal position inside one epoch.
- **Dirty fact**: a fact id whose complete raw state must be synchronized.
- **Dirty edges**: a source id whose complete outgoing edge set must be
  synchronized.
- **Base copy**: the cursor walk performed while writes continue. It is not
  called a snapshot because its pages can observe different moments.
- **Watermark**: the highest durable sequence included in one catch-up pass.
- **Named snapshot**: the destination after the write gate is closed and every
  record through the sealed final watermark is durably applied. It represents
  the source at `epoch@sequence`.
- **Cutover window**: elapsed monotonic time from closing the service gate until
  the target-backed service is installed and requests are released.

## Why the offline implementation cannot simply run beside the daemon

Four current facts are load-bearing:

1. `Database::open_impl` takes an exclusive `velesdb.lock`; a second migration
   process cannot open the source while the daemon owns it.
2. The core exposes collection-local read snapshots, but no atomic snapshot of
   facts, metadata, expiries and graph edges across all agent collections.
3. `MemoryService` has several write surfaces (`remember`, TTL and metadata
   updates, `relate`, `unrelate`, `forget`, feedback and derived autograph
   writes). None currently enters one durable migration coordinator.
4. The offline switch renames directories after source fingerprint validation.
   A live handle invalidates that proof: it may keep mutating renamed inodes.

A copied live directory, checkpoints without mutation capture, or a delta
record written only after the source mutation all leave a crash window in
which acknowledged data exists only in the source and is absent from the
destination. Those designs are rejected.

## State machine

The durable journal advances monotonically through these states:

1. `Prepared`: the epoch identity and target embedder witness are durable.
2. `Capturing`: mutation capture is enabled before the first base page.
3. `BaseCopied`: every collection cursor reached its end; writes continued.
4. `CatchingUp`: dirty records are coalesced and replayed through recorded
   watermarks while new records may arrive.
5. `CutoverReady`: observed replay capacity and backlog fit the configured
   pause budget. This is an admission verdict, not a switch.
6. `Quiescing`: the service gate is exclusive, no request is in flight, and
   the final watermark is sealed.
7. `DestinationActivated`: all records through that watermark are durable at
   the destination, the target store is verified, and the service uses the
   target embedder and store.
8. `Committed`: the old store was re-verified and retired according to the
   existing recoverable-switch rules.

Every persisted state has one recovery action. Unknown versions, impossible
transitions, a changed epoch identity, or a target witness mismatch refuse
instead of guessing.

## Mutation ordering and crash safety

Every storage mutation passes through one coordinator, including internal
entity, autograph and context writes. The coordinator holds a shared service
gate for the whole operation and follows this order while capture is active:

1. append the relevant dirty key with the next sequence;
2. flush and durability-sync the journal record;
3. apply the source mutation;
4. return the source result to the caller.

If step 1 or 2 fails, the source mutation does not run. Availability is lost
rather than data: accepting an untracked write after disk exhaustion would
make later loss inevitable. A crash after step 2 leaves a false-positive dirty
key, which replay safely resolves from the source. A crash after step 3 leaves
the same durable key, so the acknowledged mutation cannot disappear from the
migration.

The journal is append-only, checksummed, versioned and bounded on disk. Only
one epoch may capture a source at a time; a second start is a refusal, never a
second observer with a different ordering. Replay
acknowledges a watermark only after all corresponding destination writes are
durable. Compaction writes a new generation and atomically publishes it; it
never truncates the only unacknowledged copy.

No migration-only buffer may grow with the number of writes. Catch-up streams
records in bounded batches and coalesces dirty keys in bounded spillable
storage. Every request holds a shared service-generation guard so migration
start and cutover can wait for earlier requests. The no-migration path performs
no journal I/O; the guard's steady-state cost must satisfy the release gate
rather than being assumed free.

## Base copy and the named snapshot proof

Capture starts before the first cursor page. The base copy may race a write in
four ways, all converging to the same result:

- a write after an id was copied is repaired by its dirty record;
- a write before an id is copied is observed by the copy and safely replayed;
- a delete before the cursor reaches an id is absent from the copy and safely
  replayed as absent;
- an insert may be seen by both paths, but synchronization is idempotent.

Fact synchronization copies the complete raw payload, stable id and absolute
expiry, then either preserves a compatibility-proven vector or embeds current
content with the target embedder. An absent source fact deletes the destination
fact. Edge synchronization replaces one source id's complete outgoing edge set
with the source's current set; endpoint absence is a retryable dependency until
the corresponding fact records are applied.

The base copy alone is never described as coherent. Coherence is established
only in `Quiescing`: the exclusive gate waits for earlier requests, seals the
final sequence, replays through it, and prevents later source writes. The
verified destination is then the named snapshot `epoch@final_sequence`.

## Catch-up, non-convergence and cutover budget

Each catch-up pass publishes:

- input and output watermarks;
- distinct dirty facts and edge sources applied;
- journal bytes pending;
- arrival and replay rates;
- elapsed time and the largest observed apply latency.

The operator supplies a maximum cutover duration. `CutoverReady` requires a
conservative estimate derived from observed replay throughput to fit that
budget. The final phase enforces the budget with a monotonic deadline. If the
journal cannot be drained and verified before it, activation does not start:
the source stays authoritative, the gate reopens, and catch-up resumes.

If backlog grows across the configured observation window, or writes arrive at
least as fast as replay can durably apply them, the run reports
`NonConverging` with the measurements. It never loops forever and never labels
an arbitrary pause as short.

## Cutover ownership

The exclusive service gate covers reads and writes during the final window so
no request retains the old database or embedder. The daemon then:

1. seals and drains the final watermark;
2. validates destination identity, target provenance and journal completeness;
3. drops the source handle before any rename;
4. performs the existing recoverable two-rename switch;
5. opens the canonical path with the target dimension;
6. installs the target store and embedder as one active service generation;
7. releases requests and records the measured window.

If any pre-activation step fails or the deadline expires, the source is
reopened as the active generation. After activation, recovery follows the
existing fingerprinted switch table; an archive is never deleted while a live
handle can still write to it.

## Control plane and secrets

The operation is submitted to the process that already owns the source. A CLI
may be a client of that control plane, but it must not bypass the daemon's
exclusive lock. Status, refusal, cancellation before quiescing, recovery and
the measured cutover window are durable job state, not log-only text.
Cancellation is accepted only while the source is authoritative; it verifies
the epoch-owned destination before removing any artefact. A request arriving
from `Quiescing` onward reports the durable recovery action instead of trying
to roll the state machine backward.

The control surface reuses the daemon's configured transport and authorization
boundary. It does not add an unauthenticated listener. The destination and
journal are epoch-owned siblings on the source filesystem; pre-existing data
or a cross-filesystem destination is refused before capture starts.

The target backend is selected from the daemon's supported embedder registry.
Credentials remain in environment-backed configuration and are never written
to the migration journal. The target model name, dimension and vector witness
are durable because they are needed to reject a resume across an in-place model
change.

## Required implementation slices

Implementation is split by invariant. Each slice has its own tests and PR; none
closes #1796 or advertises online migration until the final end-to-end slice is
green.

1. **Coordinator and mutation census**: one service generation gate, one
   exhaustive registry of every primitive mutation, and a guard that fails
   when a new mutation bypasses capture.
2. **Durable dirty-state journal**: epoch identity, sequences, checksums,
   pre-mutation sync, recovery, bounded compaction and fault injection at every
   durability boundary.
3. **Base copy and catch-up**: fuzzy cursor walk, target re-embedding, fact and
   edge synchronization, watermark replay, bounded coalescing and progress
   metrics.
4. **Convergence controller**: measured admission, explicit non-convergence,
   cancellation and the enforced cutover deadline.
5. **Live cutover**: service-generation swap, target embedder handoff and the
   existing filesystem recovery table adapted so no live handle survives a
   rename.
6. **Control surface and release gate**: resumable job API, operator guide,
   process-level concurrent-write journeys and performance evidence. This is
   the only slice allowed to close #1796.

## Release gates

The feature remains unavailable until all of these are demonstrated:

- crash injection before and after journal append, sync, source mutation,
  destination apply, watermark acknowledgement and both switch renames;
- concurrent create, overwrite, metadata/TTL update, delete, relate, unrelate,
  feedback, autograph and extracted-memory writes on both sides of a cursor;
- equality of live facts, raw metadata, absolute expiries and complete edge
  tuples at the sealed watermark, with no duplicate application;
- restart from every durable state and refusal of corrupt, future-version or
  identity-mismatched journals;
- disk-full refusal before an unjournalled source write;
- bounded memory under a journal larger than RAM;
- non-convergence detection under a writer faster than replay;
- measured cutover both below budget and deliberately forced above budget,
  proving that the latter reopens the source without activation;
- no measurable steady-state regression when no migration is active, and
  published capture/replay overhead when one is active;
- the full repository quality, security, duplication and complexity gates.

Until then, the supported operational instruction remains: stop the daemon and
use the offline migration documented in
[`MIGRATE_EMBEDDINGS.md`](../guides/MIGRATE_EMBEDDINGS.md).
