# Nexus API Balancer — Plan

Generated: 2026-06-02

## Severity Key

| Label | Meaning |
|-------|---------|
| **P0** | Data loss, security hole, crash, or broken core workflow |
| **P1** | Likely correctness bug or major regression |
| **P2** | Edge-case bug, missing validation, or meaningful test gap |
| **P3** | Maintainability, clarity, or test-only gap |

---

## 1. Critical Bugs (P0)

### 1.1 MCP-imported keys invisible to proxy (`mcp.rs` / `api.rs` / `lib.rs`)

`create_router` clones `pools` into both `AppState.pools` and `BalancerMcpServer.pools` as separate `HashMap<String, KeyPool>`. When `mcp.import_key()` adds keys to `self.pools`, the `state.pools` in `AppState` — used by every proxy handler — is **never updated**.

**Fix:** Share pools via `Arc<RwLock<HashMap<String, KeyPool>>>` or use a channel to notify the router to refresh pool references.

### 1.2 Path traversal in `SecretStorage` (`storage.rs:18-33`)

`load_secret` and `save_secret` call `base_path.join(name)` with zero validation. No check for `..` components or absolute paths. A caller passing `"../../etc/passwd"` reads/writes arbitrary files. `config.rs:85-87` already implements the correct check with `Component::ParentDir`.

**Fix:** Reject names containing `Component::ParentDir` or `Component::RootDir`, identical to the pattern in `config.rs`.

### 1.3 Hardcoded default admin API key (`api.rs:329-331`)

```rust
.unwrap_or_else(|| "admin-secret-key-2026".to_string());
```

If `admin_key` is unset and `ADMIN_API_KEY` env var is absent, the server falls back to a well-known string baked into the binary. Anyone with this key can call every admin endpoint.

**Fix:** Return an error / refuse to start; do not fall back to a default.

### 1.4 `unwrap()` in `tokio::spawn` silently crashes (`api.rs:1441,1514,1520,1527,1544`)

Pattern:
```rust
let pool = state.pools.get(&pool_name).cloned().unwrap();
```
Panics inside `tokio::spawn` are silently swallowed by Tokio. If a pool is removed between handler execution and the spawned future, the server silently drops the request.

**Fix:** Convert all `unwrap()` calls inside spawned tasks to proper `match` / `if let Some(…)` with error-path responses or early returns.

### 1.5 Optimistic rate-limit increment before upstream request (`core.rs:151-153`)

`try_use()` increments `requests_this_second` and `requests_today` **before** the upstream request is sent. If the upstream fails, counters are never decremented, permanently skewing toward rejection.

**Fix:** Increment after a successful upstream response, or decrement on failure. Requires careful handling of concurrent state.

### 1.6 Mutex poisoning risk (`core.rs:79,84,88,158,168`)

Multiple sites call `self.inner.lock().unwrap()`. If any thread panics while holding the lock, every subsequent caller panics, taking down the entire server.

**Fix:** Handle `PoisonError` by replacing the poisoned guard with a fresh `Mutex::new(…)` containing default/fallback state, or use `std::sync::Mutex::clear_poison`.

---

## 2. Important Issues (P1)

### 2.1 `unwrap()` on double-`Option` move (`api.rs:1266-1267`)

```rust
let target_base = &pool_cfg.unwrap().target_url;  // moves/copies out
let provider = &pool_cfg.unwrap().provider;        // second unwrap
```

Only works because `Option<&T>` is `Copy`. A refactor to owned types would cause a panic.

**Fix:** Bind once: `let pool_cfg = pool_cfg.unwrap();` then access fields.

### 2.2 `unwrap()` on `Response::builder().body(...)` (`api.rs:1507,1540`)

If body construction fails (size limit, streaming error), the server panics.

**Fix:** Convert to `into_response()` with an error status.

### 2.3 SSE parsing is fragile (`api.rs:111-133`)

Manual parsing with `split("\n\n")`, `lines()`, and `strip_prefix("data:")` fails on:
- SSE comments (lines starting with `:`)
- `id:` / `retry:` fields
- Multi-line `data:` payloads

**Fix:** Use a proper SSE parser crate (e.g., `eventsource-stream`) or handle the full SSE spec.

### 2.4 `block_in_place` + `block_on` in async context (`model_registry.rs:174-181`)

Blocks a Tokio worker thread with a synchronous `block_on` call. The silent `else { return; }` path means cache rebuild is silently skipped if called outside a Tokio runtime.

**Fix:** Make the cache-rebuild function async end-to-end, using `sqlx`'s async interface directly.

### 2.5 25 MB body buffer magic number (`api.rs:1117`)

`to_bytes(body, 25 * 1024 * 1024)` loads the entire request body into memory. Defeats streaming. Magic number should be configurable.

**Fix:** Make max body size a config parameter; consider streaming for large payloads.

### 2.6 `admin_key` not redacted in config responses (`api.rs:761-767, 797-801`)

`auth.secret` and `auth.master_key` are redacted. `auth.admin_key` is returned in plaintext.

**Fix:** Add `admin_key` to the redaction list.

### 2.7 Header values silently truncated on non-UTF-8 (`api.rs:1355-1356`)

```rust
value.to_str().unwrap_or("")
```
Non-UTF-8 headers (valid per HTTP/1.1) are silently replaced with empty strings.

**Fix:** Use `value.to_str().unwrap_or(…)` with a lossy conversion or skip the header.

### 2.8 Duplicate schema management (`db.rs:63-106` vs `migrations/0001_init.sql`)

`ensure_schema()` uses `CREATE TABLE IF NOT EXISTS`, duplicating the migration files. If they diverge, behavior is undefined.

**Fix:** Remove `ensure_schema()` and rely solely on SQLx migrations, or vice versa.

### 2.9 Token extraction arithmetic edge case (`api.rs:135-139`)

When response has usage metadata with `total < input_tokens`, the fallback branch incorrectly triggers.

**Fix:** Use the usage metadata `total` field when available, falling back to estimation only when usage is absent.

### 2.10 Retry logic limited to GET/HEAD (`api.rs:1363-1369`)

Non-streaming POST/PUT/DELETE get zero retries. No jitter on retry delay, risking thundering herd under concurrent failures.

**Fix:** Extend retries to safe-to-retry methods (or all non-streaming), add jitter.

---

## 3. Moderate Issues (P2)

### 3.1 Misleading `_secret` / `_secret_type` prefix (`core.rs:33-34`)

Leading underscore conventionally means "unused", but these fields ARE used in `api.rs:1321-1323`.

**Fix:** Rename to `secret` and `secret_type`.

### 3.2 `handle_mcp` is ~240 lines (`api.rs:818-1055`)

A monolithic function handling `initialize`, `notifications/initialized`, `tools/list`, `resources/list`, `resources/read`, `tools/call`, and fallthrough. All in one function with large `if` blocks.

**Fix:** Split into a dedicated `mcp_handler.rs` module with per-method handler functions.

### 3.3 `api.rs` is ~1550 lines

The largest file in the project, containing routes, auth extractors, proxy logic, SSE streaming, retries, config handlers, admin endpoints, and MCP handling.

**Fix:** Split by responsibility: `router.rs`, `handlers/`, `auth_extractors.rs`.

### 3.4 `handle_execute` endpoint is half-baked (`api.rs:528-582`)

Acquires key, calls `try_use()`, then releases immediately without making an upstream request. `task_name` from `ExecuteRequest` is never used.

**Fix:** Either implement the upstream task execution or remove the endpoint.

### 3.5 Hardcoded `"secrets"` path (`lib.rs:46` / `main.rs:46`)

The secrets storage directory is hardcoded as a string literal.

**Fix:** Add a `secrets_path` config option.

### 3.6 Dead code: `count` in `model_registry.rs:88`

```rust
let _ = count;
```
Assigned but immediately suppressed.

**Fix:** Remove the assignment or use the value.

### 3.7 `get_supported_providers()` is hardcoded (`config.rs:78-80`)

Adding a new provider requires code changes. Same issue with `get_standard_url`.

**Fix:** Make provider list data-driven (config file or directory scan).

### 3.8 `RawModel.metadata` as deeply nested optional tuple (`model_registry.rs:228-231`)

```rust
metadata: Option<(Option<String>, Option<i64>, Option<String>)>
```
Requires ugly nested `.unwrap().0` chains.

**Fix:** Define a proper struct with named fields.

### 3.9 BOM stripping is fragile (`storage.rs:22`)

```rust
Ok(secret.trim_start_matches('\u{feff}').trim().to_string())
```
Only strips repeated BOM at position 0. If whitespace precedes BOM, it won't be stripped.

**Fix:** Use `strip_prefix('\u{feff}')` for a single precise removal.

### 3.10 Individual INSERTs in transaction loop (`db.rs:289-310`)

For 100+ model syncs, executes individual round-trips within a transaction.

**Fix:** Batch with `INSERT INTO ... VALUES (...), (...), ... ON CONFLICT ...`.

---

## 4. Missing Tests

### 4.1 Files with zero unit tests

| File | What to test |
|------|-------------|
| `auth.rs` | `validate_token` (valid, expired, wrong secret, wrong issuer), `generate_token`, edge cases (empty claims, special chars) |
| `core.rs` | `ApiKey::try_use` (within limit, at limit, over limit, token limits), `KeyPool::acquire`/`release` (empty pool, full pool, concurrent acquire), `rate_limit` reset timing |
| `db.rs` | `log_request`, `get_stats` (empty, some data, filtering), `upsert_provider_models`, `get_models_by_pool`, `bind_client_pool`, migration state |
| `config.rs` | `load` (valid YAML, missing file, env override, bad YAML), `save` (round-trip), `get_standard_url` for all providers |
| `mcp.rs` | `list_pools`, `update_description`, `export_key`, `import_key`, error paths, duplicate key import |
| `utils.rs` | `count_tokens` (empty, short, long, non-ASCII), `estimate_*_tokens` corner cases, `verify_key` (valid, invalid, timeout, network error) |
| `lib.rs` | `run_server` with various config states |

### 4.2 Integration / E2E gaps

| Gap | Description |
|-----|-------------|
| Provider failure | No test for upstream returning 5xx during streaming |
| Auth failure paths | Invalid JWT, expired JWT, wrong issuer, missing header, admin key with wrong role |
| Concurrent key starvation | Many concurrent requests exhausting all keys in a pool |
| SSE streaming | No test for multi-frame SSE, SSE comments, malformed SSE |
| Model registry sync | Network failure during sync, partial sync, stale model eviction |
| MCP end-to-end | Import key via MCP, then proxy a request with it |
| Graceful shutdown | SIGTERM while requests are in-flight |
| Config hot-reload | PATCHing config while requests are being proxied |

### 4.3 Coverage targets

- **Line coverage:** Target >70% for `core.rs`, `auth.rs`, `storage.rs`, `config.rs`
- **Branch coverage:** Target >60% for `api.rs` utility functions, `model_registry.rs` dispatch logic
- **Integration:** 5-10 E2E scenarios covering happy path + failure modes

---

## 5. Architectural Improvements

### 5.1 Module structure

Current (flat):
```
src/
  main.rs, lib.rs, api.rs, auth.rs, config.rs, core.rs, db.rs,
  mcp.rs, mcp_client.rs, model_registry.rs, storage.rs, utils.rs
```

Proposed:
```
src/
  main.rs                          # Binary entry point (thin)
  lib.rs                           # Server orchestration (thin)
  config/                          # Config loading, validation, types
    mod.rs
    provider_defaults.rs
  auth/                            # JWT, admin key, API key auth
    mod.rs
    jwt.rs
    extractors.rs
  proxy/                           # Core proxy logic
    mod.rs
    router.rs                      # Route definitions
    handler_proxy.rs               # /proxy/:pool handler
    handler_unified.rs             # /v1/*, /v1beta/* handler
    handler_execute.rs             # /execute handler
    sse.rs                         # SSE streaming/parsing
    retry.rs                       # Retry logic
  pool/                            # Key pools, rate limiting
    mod.rs
    api_key.rs                     # ApiKey + rate limits
    key_pool.rs                    # KeyPool (acquire/release)
  admin/                           # Admin endpoints
    mod.rs
    handler_clients.rs
    handler_config.rs
    handler_stats.rs
    handler_keys.rs
  mcp/                             # MCP server
    mod.rs
    handler.rs
    tools.rs
  db/                              # Database layer
    mod.rs
    migrations.rs
    models.rs
    queries.rs
  model_registry.rs                # Model discovery & routing
  storage.rs                       # Secret file storage
  utils.rs                         # Token counting, helpers
```

### 5.2 Shared state ownership

**Problem:** `HashMap<String, KeyPool>` is cloned into two places (`AppState` and `BalancerMcpServer`), allowing them to diverge.

**Fix:** Wrap in `Arc<RwLock<HashMap<String, KeyPool>>>`. Both `AppState` and `BalancerMcpServer` hold the same `Arc`. Mutations via MCP are immediately visible to proxy handlers.

### 5.3 Error handling

- Replace `unwrap()` calls with proper error propagation
- Define a `AppError` enum implementing `IntoResponse` with structured JSON errors
- Infallible error handling inside `tokio::spawn` — log + return HTTP 500
- Handle `Mutex::PoisonError` gracefully

### 5.4 Rate-limit counter placement

Move counter increments from `ApiKey::try_use` (pre-request) to a new `ApiKey::record_success` (post-response). The acquire step should only check current limits, not increment them. Requires an atomic compare-and-swap pattern for the check.

### 5.5 Configuration

- Make `max_body_size` a config parameter
- Make `secrets_path` a config parameter
- Make provider list data-driven
- Remove hardcoded fallback secrets

### 5.6 Observability

Add structured logging at key decision points:
- Key pool acquire/release
- Rate limit deny with reason
- Model registry sync start/complete/fail
- Auth decision (accept/deny with reason)

---

## 6. Refactoring Targets

| Priority | File | Action |
|----------|------|--------|
| High | `api.rs` | Split into modules under `proxy/`, `admin/`, `auth/` |
| High | `mcp.rs` | Split into dedicated `mcp/` module |
| Medium | `core.rs` | Rename `_secret` → `secret`, `_secret_type` → `secret_type`; fix counter placement |
| Medium | `db.rs` | Remove `ensure_schema()`; batch INSERTs; extract query functions |
| Medium | `config.rs` | Make provider list data-driven |
| Low | `model_registry.rs` | Replace nested tuple with named struct |
| Low | `storage.rs` | Add path traversal validation; fix BOM stripping |
| Low | `utils.rs` | No structural changes needed |

---

## 7. Timeline / Roadmap

### Phase 1: Safety (estimated: 2-3 days)
1. P0.1 — Path traversal fix in `storage.rs`
2. P0.2 — Fix double-unwrap pattern in `api.rs`
3. P0.3 — Remove `unwrap()` in `tokio::spawn` futures
4. P0.4 — Fix `Response::builder().body().unwrap()`
5. P0.6 — Remove hardcoded admin key default
6. P1.6 — Fix Mutex poisoning in `core.rs`

### Phase 2: Correctness (estimated: 3-4 days)
1. P0.5 — Shared pool state via `Arc<RwLock<…>>`
2. P1.1 — Fix rate-limit counter placement
3. P1.2 — Fix header byte handling
4. P1.3 — Fix admin_key redaction
5. P1.4 — Fix token extraction arithmetic
6. P1.5 — Fix duplicate schema management

### Phase 3: Architecture (estimated: 4-5 days)
1. Split `api.rs` into modules
2. Split `mcp.rs` into dedicated module
3. Create structured error types
4. Move SSE parsing to proper crate
5. Make body size, secrets path, provider list configurable

### Phase 4: Tests (estimated: 5-7 days)
1. Unit tests for `core.rs`, `auth.rs`, `config.rs`, `db.rs`, `utils.rs`, `mcp.rs`
2. Integration tests for SSE streaming, auth failure paths, provider failures
3. E2E tests for MCP import + proxy flow
4. E2E tests for concurrent key starvation
5. E2E tests for config hot-reload

---

## 8. Files Summary

| File | Issues | Lines | Priority |
|------|--------|-------|----------|
| `src/api.rs` | P0.2, P0.3, P0.4, P0.6, P1.4, P1.6, P1.7, P1.8, P2.1, P2.2, P2.7, P2.10, P2.13 | 1548 | Critical |
| `src/core.rs` | P0.5, P1.2, P1.6, P2.1, P2.11, P2.13 | 200 | Critical |
| `src/storage.rs` | P0.1 | 109 | Critical |
| `src/mcp.rs` | P0.5 | 215 | Critical |
| `src/lib.rs` | P0.5 | 138 | Critical |
| `src/model_registry.rs` | P1.5, P1.7, P2.6, P2.13 | 415 | High |
| `src/db.rs` | P1.8, P2.4, P2.5 | 314 | High |
| `src/config.rs` | P2.8 | 116 | Medium |
| `src/auth.rs` | None | 75 | Low |
| `src/utils.rs` | None | 160 | Low |
| `src/mcp_client.rs` | None | 110 | Low |
| `src/main.rs` | None | 47 | Low |
