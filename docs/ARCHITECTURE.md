# Architecture

nexus_balancer is a high-performance Rust proxy server and intelligent key balancer for AI providers.

## Overview

```
Client → Axum HTTP Server → Auth Layer → Proxy/Balancer → Upstream AI Provider
                                        → MCP Server
                                        → Model Registry
```

## Entry Point

The server starts via `run_server()` in `lib.rs`:

1. Loads config, secrets, DB
2. Creates `KeyPool` instances from pool config (auto-calculates capacity)
3. Initializes `ModelRegistry` and spawns periodic model sync (every 6h)
4. Creates `AuthManager`
5. Builds Axum router with CORS layer
6. Binds TCP listener and starts serving

## Core Components

| Component | File | Purpose |
|-----------|------|---------|
| **Server** | `lib.rs` | Initialization, `run_server()`, CORS layer, startup banner |
| **Router** | `api.rs` | All REST routes, request handlers, auth extractors, proxy logic |
| **Config** | `config.rs` | YAML-based configuration loading with env override support |
| **Auth** | `auth.rs` | JWT token validation and generation (HS256) |
| **Key Pool** | `core.rs` | Bounded channel-based key pool with per-key rate limiting (RPS, RPD, TPM, TPD) |
| **Database** | `db.rs` | SQLite via SQLx with migrations. Logs requests, stores models, manages client-pool bindings |
| **MCP Server** | `mcp.rs` | JSON-RPC over HTTP for programmatic pool and key management |
| **MCP Client** | `mcp_client.rs` | stdio-to-HTTP bridge for local MCP client tools |
| **Model Registry** | `model_registry.rs` | Auto-discovers models via provider `/models` endpoints, cached for O(1) lookup |
| **Storage** | `storage.rs` | File-based API key storage with path traversal protection |
| **Utils** | `utils.rs` | Token counting (tiktoken), key verification, response parsing |

## Data Flow

1. Client sends request to Axum HTTP server
2. `AuthToken`/`AdminToken` extractor validates JWT or API key (or admin key bypass)
3. Router matches path to handler (`proxy`, `unified`, `mcp`, `admin`, etc.)
4. Unified gateway (`/v1/*`) resolves model via: (a) explicit `//provider//model` prefix, (b) Model Registry O(1) lookup, (c) heuristic fallback
5. Proxy handler acquires key from pool, forwards request to upstream provider
6. SSE responses are streamed with tokio channels; non-SSE bodies are buffered
7. Failed requests retry up to 2 times (non-streaming GET/HEAD only) with exponential backoff (500ms, 1000ms)
8. Usage is logged to SQLite; tokens accounted to key rate limits

## Key Pool Design

Each `KeyPool` is an `async-channel` bounded queue of `ApiKey` slots. Multiple slots per key enable concurrency. Rate limits are enforced per key in `ApiKey::try_use()` with automatic cooldown support.

## KV Cache (Google Gemini)

When `kv_cache` is enabled for a client on a Google pool, requests are automatically upgraded from `/v1/` to `/v1beta/` for context caching support. The flag is stored per client per pool in `client_pools.kv_cache`.

## Unified Proxy

`handle_unified_proxy()` (api.rs:1121) routes by model:
1. Parse `model` field from request body or `/models/{name}` path
2. Check for explicit `//provider//model` prefix
3. Query Model Registry for pool with highest priority
4. Fallback to heuristic prefix matching (gpt- → openai, claude- → anthropic, etc.)
5. Fallback to first allowed pool
6. Delegate to `handle_proxy_internal()`

## Threading Model

Single Tokio runtime. `ArcSwap` for lock-free config reloading. `RwLock` for model cache. `Mutex` for per-key state counters.
