# Authentication & Authorization

## Auth Flow

1. Client presents a JWT Bearer token or admin key
2. `AuthToken` extractor validates the token
3. Claims determine role (`admin` or `client`)
4. Admin endpoints require `AdminToken` extractor (role check for `admin`)

### Token resolution order

1. If `auth.enabled: false` → all requests treated as admin (`role: "admin"`, sub: `"local-client"`)
2. `X-Admin-Key` header → checked against `auth.admin_key` or `ADMIN_API_KEY` env var (fallback default: `"admin-secret-key-2026"`)
3. Master key (if configured) → checked against `auth.master_key`, assigns admin role, client ID from `X-Nexus-Client-Id` header
4. JWT Bearer token → validated via `AuthManager::validate_token()`
5. API key headers fallback → `x-goog-api-key`, `x-api-key`, `api-key`, `key=` query param

## Token Types

### JWT Bearer Token
- Signed with HS256 using `auth.secret`
- Contains: `sub` (client ID), `exp`, `iss`, `aud`, `role`
- Validated against configured `issuer` and `audience`
- Generated via `POST /admin/clients` or `POST /auth/register`

### Admin Key
- Passed via `X-Admin-Key` header
- Configured in `auth.admin_key` or `ADMIN_API_KEY` env var
- Bypasses JWT validation entirely
- **Hardcoded default**: `"admin-secret-key-2026"` if neither config nor env var is set

### Master Key
- Configured in `auth.master_key`
- Bypasses JWT validation, assigns admin role
- Client ID extracted from `X-Nexus-Client-Id` header

### API Key Headers
Fallback authentication via:
- `x-goog-api-key`
- `x-api-key`
- `api-key`
- `key=` query parameter

## Auth Bypass

When `auth.enabled: false`, all requests are treated as admin (for local development).

## Client-Pool Authorization

Clients are restricted to specific pools via the `client_pools` database table. Non-admin clients can only proxy to explicitly allowed pools.

## Key Import Authorization

`POST /admin/keys/{pool}` uses `AuthToken` (not `AdminToken`):
- Admin users can import keys to any pool
- Non-admin users can import keys only to pools they are authorized for
- The MCP `tools/call` handler applies the same logic

## KV Cache

Per-client, per-pool KV cache flag stored in `client_pools.kv_cache`. When enabled on Google pools, requests are automatically upgraded to `/v1beta/` for context caching support.
