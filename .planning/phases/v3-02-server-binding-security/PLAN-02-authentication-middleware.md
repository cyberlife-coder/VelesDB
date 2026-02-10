---
phase: v3-02-server-binding-security
plan: 02
name: authentication-middleware
wave: 1
depends_on: [01]
autonomous: true
parallel_safe: false
---

# Plan 02: Authentication Middleware

## Objective

Add optional API key authentication to the server. When `VELESDB_API_KEY` is set, all endpoints (except `/health` and `/swagger-ui`) require `Authorization: Bearer <key>`. When unset, the server runs in dev mode with no auth.

## Context

- **Requirement:** ECO-03 (Server: No authentication/authorization)
- **Phase goal:** Add authentication and runtime safety
- **Current state:** Zero authentication — all endpoints publicly accessible
- **Design:** Axum middleware layer, extracted before routing. Tower-compatible.

## Tasks

### Task 1: Create Auth Middleware Module

**Files:**
- `crates/velesdb-server/src/handlers/auth.rs` (NEW)
- `crates/velesdb-server/src/handlers/mod.rs` (UPDATE)

**Action:**
1. Create `auth.rs` with:
   - `ApiKeyAuth` middleware (tower Layer + Service pattern, or Axum `middleware::from_fn`).
   - Read `VELESDB_API_KEY` from environment at startup (passed via `AppState` or extracted once).
   - If env var is set:
     - Extract `Authorization: Bearer <token>` header.
     - Compare token with stored key using **constant-time comparison** (`subtle` crate or manual).
     - Return `401 Unauthorized` with JSON `ErrorResponse` if missing/invalid.
   - If env var is NOT set:
     - Pass through all requests (dev mode).
   - Skip auth for: `/health`, `/swagger-ui/*`, `/api-docs/*`.
   - Log at startup: whether auth is enabled or disabled (dev mode warning).
2. Add `pub mod auth;` to `handlers/mod.rs`.
3. Re-export middleware from `handlers/mod.rs`.

**What to avoid:**
- Do NOT hardcode the API key anywhere.
- Do NOT use string equality (`==`) for key comparison — use constant-time comparison to prevent timing attacks.
- Do NOT block `/health` endpoint — monitoring must always work.

**Verify:**
```powershell
cargo check -p velesdb-server
```

**Done when:**
- `auth.rs` exists with middleware implementation.
- Constant-time comparison is used for key validation.
- Module is registered in `handlers/mod.rs`.

### Task 2: Wire Middleware into Router

**Files:**
- `crates/velesdb-server/src/main.rs`
- `crates/velesdb-server/src/lib.rs`
- `crates/velesdb-server/Cargo.toml`

**Action:**
1. In `main.rs`:
   - Read `VELESDB_API_KEY` from env var at startup.
   - Store the optional key in `AppState` (add `api_key: Option<String>` field).
   - Apply auth middleware layer to `api_router` (after route definition, before CORS/Trace).
   - Log whether auth is enabled: `tracing::info!("Authentication: enabled")` or `tracing::warn!("Authentication: DISABLED (dev mode)")`.
2. In `lib.rs`:
   - Update `AppState` struct to include `api_key: Option<String>`.
   - Re-export auth middleware if needed externally.
3. In `Cargo.toml`:
   - Add `subtle = "2"` dependency if using `subtle::ConstantTimeEq` (preferred).
   - Or implement constant-time comparison manually (avoid external dep).

**Verify:**
```powershell
cargo build -p velesdb-server
```

**Done when:**
- Auth middleware is applied to the router.
- `AppState` carries the optional API key.
- Startup logs show auth status.

### Task 3: Write Auth Tests

**Files:**
- `crates/velesdb-server/tests/auth_tests.rs` (NEW)

**Action:**
1. Create test file with:
   - `test_health_no_auth_required` — `/health` returns 200 even with API key configured.
   - `test_unauthenticated_request_returns_401` — Request without `Authorization` header returns 401 when key is set.
   - `test_wrong_api_key_returns_401` — Request with wrong Bearer token returns 401.
   - `test_correct_api_key_returns_200` — Request with correct Bearer token succeeds.
   - `test_no_auth_in_dev_mode` — When `VELESDB_API_KEY` is not set, all requests pass through.
   - `test_malformed_auth_header_returns_401` — `Authorization: Basic ...` or `Authorization: Bearer` (empty) returns 401.
2. Use `axum::test` helpers or build test app with known API key.

**Verify:**
```powershell
cargo test -p velesdb-server --test auth_tests
```

**Done when:**
- All 6 auth tests pass.
- 401 response includes JSON body with error message.

## Overall Verification

```powershell
cargo check -p velesdb-server
cargo clippy -p velesdb-server -- -D warnings
cargo test -p velesdb-server
```

## Success Criteria

- [ ] Unauthenticated request → 401 when `VELESDB_API_KEY` is set
- [ ] Wrong key → 401
- [ ] Correct key → 200 (pass through to handler)
- [ ] No key env var → dev mode (all requests pass)
- [ ] `/health` always accessible regardless of auth
- [ ] Constant-time key comparison (no timing attack)
- [ ] All server tests pass

## Parallel Safety

- **Exclusive write files:** `handlers/auth.rs` (new), `main.rs`, `lib.rs`, `handlers/mod.rs`, `Cargo.toml`, `tests/auth_tests.rs` (new)
- **Shared read files:** None
- **Conflicts with:** Plan 01 (both write `main.rs`, `lib.rs`, `handlers/mod.rs`) → execute after Plan 01

## Output

- **Created:** `handlers/auth.rs`, `tests/auth_tests.rs`
- **Modified:** `handlers/mod.rs`, `main.rs`, `lib.rs`, `Cargo.toml`
