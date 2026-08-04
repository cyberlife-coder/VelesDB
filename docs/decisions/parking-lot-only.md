# Locks are parking_lot, never std::sync

Status: accepted

Every lock in the workspace comes from `parking_lot`. `std::sync::Mutex` and
`std::sync::RwLock` are not used.

**Why.** `std` locks poison on panic. A poisoned lock turns a fault in one
operation into a permanent failure of every later one that touches the same
data, which is a worse outcome than the original panic for an embedded database
the caller cannot restart independently.

**Evidence.** Stated as non-negotiable in [`AGENTS.md`](../../AGENTS.md); the
lock ordering that goes with it is in
[`CONCURRENCY_MODEL.md`](../CONCURRENCY_MODEL.md).
