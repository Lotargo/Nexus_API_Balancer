# Database Schema

SQLite via SQLx. Migrations in `migrations/`.

## Tables

### `request_logs`

| Column | Type | Description |
|--------|------|-------------|
| id | INTEGER PK | Auto-increment |
| client_id | TEXT | JWT `sub` claim |
| key_id | TEXT | API key ID used |
| pool_id | TEXT | Pool name |
| status | TEXT | `success`, `rate_limited`, `client_disconnected`, HTTP status string, etc. |
| latency_ms | INTEGER | Request duration in ms |
| tokens_used | INTEGER | Total tokens consumed |
| error_message | TEXT | Error details on failure |
| request_ip | TEXT | Client IP address |
| created_at | TEXT | ISO-8601 timestamp |

### `clients`

| Column | Type | Description |
|--------|------|-------------|
| id | TEXT PK | UUID or custom ID |
| name | TEXT | Human-readable name |
| auth_token | TEXT | JWT token issued at registration |
| status | TEXT | `active` (default) |
| created_at | TEXT | ISO-8601 timestamp |

### `client_pools`

| Column | Type | Description |
|--------|------|-------------|
| client_id | TEXT | FK → clients(id) ON DELETE CASCADE |
| pool_id | TEXT | Pool name |
| kv_cache | BOOLEAN | KV cache enabled flag |
| created_at | TEXT | ISO-8601 timestamp |

PK: `(client_id, pool_id)`

### `provider_models`

| Column | Type | Description |
|--------|------|-------------|
| id | INTEGER PK | Auto-increment |
| provider_name | TEXT | Provider (openai, gemini, etc.) |
| pool_name | TEXT | Pool that discovered this model |
| model_id | TEXT | Model identifier |
| owned_by | TEXT | Provider owner string |
| context_window | INTEGER | Max context length |
| capabilities | TEXT | JSON string of model capabilities (reserved) |
| is_stale | BOOLEAN | Marked stale before resync, deleted after 24h |
| fetched_at | TEXT | Last fetch timestamp |
| created_at | TEXT | Creation timestamp |

Indexes: `model_id`, `provider_name`. UNIQUE on `(provider_name, model_id)`.

## Key Constraints

1. `client_pools.client_id` references `clients(id)` with `ON DELETE CASCADE`
2. `PRAGMA foreign_keys = ON` is set on every connection
3. Requests to non-allowed pools return 403 for non-admin clients
