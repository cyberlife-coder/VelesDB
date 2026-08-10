# Core must never reference a premium crate

Status: accepted

`velesdb-core` ships only the policy-free port — `DatabaseObserver`,
`on_query_request` returning an `AccessDecision`, and `open_with_observer` —
with an allow-all, no-op default. The enforcing policy (RBAC, tenancy, audit)
lives in `velesdb-private` as an observer implementation.

**Why.** The open-core split is a licence boundary, not a matter of taste. One
premium symbol reachable from core would make the published crate carry code it
cannot ship under its own licence, and the leak would be invisible until
someone read the dependency tree.

**Evidence.** The port is `crates/velesdb-core/src/observer/`. CI refuses the
crossing rather than trusting review: the `node-binding-tests` job in
`.github/workflows/ci.yml` fails if `cargo tree -p velesdb-node` resolves
`velesdb-core`.
