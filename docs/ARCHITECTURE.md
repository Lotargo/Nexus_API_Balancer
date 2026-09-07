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
4. Unified gateway (`/v1/*`) detects request capability and model. JSON chat requests use existing model routing; multipart audio transcription/translation requests use `stt` capability routing.
5. Capability routing builds an ordered pool candidate list by model, capability, client access, and pool priority.
6. Proxy handler acquires a key from the selected pool and forwards the original request body and content type to the upstream provider.
7. STT requests can fail over across provider pools after 429, 5xx, timeout, or transport failure. Explicit `//provider//model` routing does not escape the selected provider.
8. Same-pool GET/HEAD retries keep the existing exponential backoff behavior.
9. SSE responses are streamed with tokio channels; non-SSE bodies are buffered.
10. Usage is logged to SQLite; tokens accounted to key rate limits.

## Key Pool Design

Each `KeyPool` is an `async-channel` bounded queue of `ApiKey` slots. Multiple slots per key enable concurrency. Rate limits are enforced per key in `ApiKey::try_use()` with automatic cooldown support.

## KV Cache (Google Gemini)

When `kv_cache` is enabled for a client on a Google pool, requests are automatically upgraded from `/v1/` to `/v1beta/` for context caching support. The flag is stored per client per pool in `client_pools.kv_cache`.

## Unified Proxy

`handle_unified_proxy()` routes by model and capability:

1. Detect `chat` or `stt` from the request path.
2. Parse `model` from JSON or multipart form data.
3. Check for explicit `//provider//model` routing and rewrite the forwarded model field.
4. Resolve candidate pools through Model Registry plus configured pool capabilities.
5. Sort candidates by priority.
6. For chat, keep conservative single-provider behavior.
7. For STT, try the next eligible provider on retriable upstream failure.
8. Delegate each attempt to `handle_proxy_internal()`.

Provider-side 429 responses place the key in a short cooldown. 5xx and transport failures use a shorter cooldown before the pool is considered again.

## Threading Model

Single Tokio runtime. `ArcSwap` for lock-free config reloading. `RwLock` for model cache. `Mutex` for per-key state counters.
