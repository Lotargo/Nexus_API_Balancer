# Architecture

nexus_balancer is a high-performance Rust proxy server and intelligent key balancer for AI providers.

## Overview

```
Client → Axum HTTP Server → Auth Layer → Proxy/Balancer → Upstream AI Provider
                                        → MCP Server
                                        → Model Registry
```

## Core Components

| Component | File | Purpose |
|-----------|------|---------|
| **HTTP Server** | `main.rs` | Axum-based entry point. Initializes config, DB, pools, and starts the listener |
| **Router** | `api.rs` | All REST routes, request handlers, auth extractors, proxy logic |
| **Config** | `config.rs` | YAML-based configuration loading with env override support |
| **Auth** | `auth.rs` | JWT token validation and generation (OAuth 2.1 Bearer) |
| **Key Pool** | `core.rs` | Bounded channel-based key pool with per-key rate limiting (RPS, RPD, TPM, TPD) |
| **Database** | `db.rs` | SQLite via SQLx with migrations. Logs requests, stores models, manages client-pool bindings |
| **MCP Server** | `mcp.rs` | JSON-RPC over HTTP for programmatic pool and key management |
| **MCP Client** | `mcp_client.rs` | stdio-to-HTTP bridge for local MCP client tools |
| **Model Registry** | `model_registry.rs` | Auto-discovers models via provider `/models` endpoints, cached for O(1) lookup |
| **Storage** | `storage.rs` | File-based API key storage (one key per line, client-isolated directories) |
| **Utils** | `utils.rs` | Token counting (tiktoken), key verification, response parsing |

## Data Flow

1. Client sends request to Axum HTTP server
2. `AuthToken`/`AdminToken` extractor validates JWT or API key
3. Router matches path to handler (`proxy`, `unified`, `mcp`, `admin`, etc.)
4. Proxy handler acquires key from pool, forwards request to upstream provider
5. Response is streamed back; usage is logged to SQLite

## Key Pool Design

Each `KeyPool` is an `async-channel` bounded queue of `ApiKey` slots. Multiple slots per key enable concurrency. Rate limits are enforced per key in `ApiKey::try_use()` with automatic cooldown support.

## Threading Model

Single Tokio runtime. ArcSwap for lock-free config reloading. RwLock for model cache. Mutex for per-key state.
