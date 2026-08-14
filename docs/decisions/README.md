# Decisions

One decision per file: what was decided, why, and the code, PR or CI job that
proves it. See also the [docs index](../README.md) for the full documentation
map.

Entries are not edited into agreement with later thinking. A decision that no
longer holds gets a new file that supersedes it, so the reason the old one
looked right at the time survives.

| Decision | Summary |
|----------|---------|
| [Core must never reference a premium crate](./core-premium-boundary.md) | The open-core split is a licence boundary, enforced by a `cargo tree` assertion |
| [Locks are parking_lot, never std::sync](./parking-lot-only.md) | `std` locks poison on panic, turning one fault into permanent failure |
| [Tests run single-threaded](./tests-single-threaded.md) | The suites share filesystem state |
| [Repository scripts are tested with stdlib unittest](./unittest-not-pytest.md) | The gate jobs use a bare `setup-python`; a skipped guard reads like a passing one |
| [The context optimizer refuses high-risk restitution](./optimizer-refuses-high-risk.md) | An unrestorable compiled view is worse than a large one |
| [The shared daemon is reached over native HTTP](./native-http-over-bridge.md) | The bridge does not recover from an idle-expired session |
| [A repository edit requires a successful causal recall first](./recall-before-edit.md) | Bound to the exact session and checkout, written only after success |
| [Tool results are replaced on one host only](./no-auto-replacement-on-a-second-host.md) | Replacement needs an output contract; parity without one is a claim |
| [Online memory migration is a daemon-owned dirty-state protocol](./online-memory-migration-contract.md) | A durable pre-mutation journal turns a live base copy into a named cutover snapshot |
