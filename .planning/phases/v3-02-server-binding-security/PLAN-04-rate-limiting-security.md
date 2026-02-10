---
phase: v3-02-server-binding-security
plan: 04
name: rate-limiting-security
wave: 2
depends_on: [02]
autonomous: true
parallel_safe: true
---

# Plan 04: Rate Limiting + Security Hardening

## Objective

Add configurable rate limiting to the server (default 100 req/s per IP) and harden CORS configuration. Returns HTTP 429 when rate exceeded.

## Context

- **Requirement:** ECO-14 (Server: No rate limiting)
- **Phase goal:** Runtime safety and production hardening
- **Current state:** Zero rate limiting. CORS is `CorsLayer::permissive()` (allows all origins).
- **Depends on:** Plan 02 (auth) — rate limiting should apply after auth middleware
- **Options:** `tower::limit::RateLimit` (simple), `tower-governor` (per-IP, configurable). Prefer `tower-governor` for per-IP granularity.

## Tasks

### Task 1: Add Rate Limiting Middleware

**Files:**
- `crates/velesdb-server/src/handlers/rate_limit.rs` (NEW)
- `crates/velesdb-server/src/handlers/mod.rs` (UPDATE)
- `crates/velesdb-server/Cargo.toml` (UPDATE)

**Action:**
1. Add `tower-governor = "0.4"` to `Cargo.toml` dependencies.
2. Create `rate_limit.rs` with:
   - Configuration struct reading from env vars:
     - `VELESDB_RATE_LIMIT`: requests per second per IP (default: 100).
     - `VELESDB_RATE_BURST`: burst capacity (default: 50).
   - Builder function returning a `GovernorLayer` configured with:
     - Per-IP keying (extract from `ConnectInfo` or `X-Forwarded-For` header).
     - Configurable rate from env var.
   - Log rate limit configuration at startup.
3. Add `pub mod rate_limit;` to `handlers/mod.rs`.

**What to avoid:**
- Do NOT apply rate limiting to `/health` — monitoring probes must never be rate-limited.
- Do NOT hardcode rate values — always read from env with sensible defaults.
- Do NOT use `tower::limit::RateLimit` alone — it's global, not per-IP.

**Verify:**
```powershell
cargo check -p velesdb-server
```

**Done when:**
- `rate_limit.rs` exists with `GovernorLayer` configuration.
- Rate limit values come from environment variables.
- Module registered in `handlers/mod.rs`.

### Task 2: Wire Rate Limiting + Harden CORS in Router

**Files:**
- `crates/velesdb-server/src/main.rs`
- `crates/velesdb-server/src/lib.rs`

**Action:**
1. In `main.rs`:
   - Apply rate limiting layer to `api_router` (after auth, before CORS).
   - Read `VELESDB_CORS_ORIGIN` env var:
     - If set → use specific allowed origin(s) (comma-separated).
     - If not set → use `CorsLayer::permissive()` with a `tracing::warn!` log.
   - Ensure `/health` is on a separate sub-router WITHOUT rate limiting.
   - Log rate limit config: `tracing::info!("Rate limit: {} req/s per IP", rate)`.
2. In `lib.rs`:
   - Add rate limit config fields to `AppState` if needed (or keep separate).

**Verify:**
```powershell
cargo build -p velesdb-server
```

**Done when:**
- Rate limiting middleware applied to API routes.
- `/health` excluded from rate limiting.
- CORS configurable via env var (permissive only in dev mode with warning).

### Task 3: Write Rate Limiting Tests

**Files:**
- `crates/velesdb-server/tests/rate_limit_tests.rs` (NEW)

**Action:**
1. Create test file with:
   - `test_rate_limit_returns_429` — Send burst of requests exceeding limit, verify 429 response.
   - `test_health_not_rate_limited` — `/health` returns 200 even under rate limit pressure.
   - `test_rate_limit_response_body` — 429 response includes JSON body with meaningful error message.
   - `test_rate_limit_headers` — Response includes `Retry-After` or `X-RateLimit-*` headers.
2. Use small rate limit (e.g., 2 req/s) in tests for fast verification.

**Verify:**
```powershell
cargo test -p velesdb-server --test rate_limit_tests
```

**Done when:**
- All rate limit tests pass.
- 429 response includes descriptive JSON error.

## Overall Verification

```powershell
cargo check -p velesdb-server
cargo clippy -p velesdb-server -- -D warnings
cargo test -p velesdb-server
```

## Success Criteria

- [ ] Rate limiting active with configurable rate (default 100 req/s per IP)
- [ ] Exceeding rate → 429 Too Many Requests with JSON body
- [ ] `/health` excluded from rate limiting
- [ ] CORS configurable via `VELESDB_CORS_ORIGIN` env var
- [ ] Permissive CORS only in dev mode with warning log
- [ ] All server tests pass

## Parallel Safety

- **Exclusive write files:** `handlers/rate_limit.rs` (new), `tests/rate_limit_tests.rs` (new), `main.rs`, `Cargo.toml`
- **Shared read files:** None
- **Conflicts with:** Plan 05 (both may touch `main.rs` for CORS) → coordinate. Plan 05 focuses on clippy/tests, not CORS.

## Output

- **Created:** `handlers/rate_limit.rs`, `tests/rate_limit_tests.rs`
- **Modified:** `handlers/mod.rs`, `main.rs`, `lib.rs`, `Cargo.toml`
