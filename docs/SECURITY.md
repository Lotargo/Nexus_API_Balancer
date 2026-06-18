# Security

## Authentication

- **JWT Bearer tokens** (HS256) validated by `AuthToken` extractor in `api.rs`
- **Admin key** (`X-Admin-Key` header) bypasses JWT — configured via `auth.admin_key` or `ADMIN_API_KEY` env var
- **Master key** (`auth.master_key`) bypasses JWT, assigns admin role; client ID extracted from `X-Nexus-Client-Id` header
- **API key fallback**: `x-goog-api-key`, `x-api-key`, `api-key` headers or `key=` query param
- When `auth.enabled: false`, all requests are treated as admin (for local development only)

## Hardcoded Defaults

If `auth.admin_key` is not set and `ADMIN_API_KEY` env var is not set, the server falls back to `"admin-secret-key-2026"`. This default must be overridden in production.

## CORS

By default, only `http://localhost:3317` is allowed. Configure via `server.cors_allowed_origin` in config.yaml or `CORS_ALLOWED_ORIGIN` env var.

## Path Traversal Protection

Secret file names, config paths, client IDs, and provider names are validated by `is_safe_name()` in `storage.rs`:

- Rejects absolute paths (`/`, `C:\`)
- Rejects parent directory components (`..`)
- Only allows single `Normal` path segments

Applied in:
- `SecretStorage::load_secret()`, `save_secret()`, `save_secret_for_client()`
- `AppConfig::load()`, `save()`

## Key Isolation

- API keys stored in `secrets/{client_id}/{provider}_api_key.txt`
- Each client's keys are physically separated at the filesystem level
- Non-admin clients can only proxy to explicitly allowed pools (`client_pools` DB table)

## SQLite Security

- `PRAGMA foreign_keys = ON` — cascade deletes from `clients` to `client_pools`
- No raw SQL interpolation — all queries use SQLx parameterized statements
