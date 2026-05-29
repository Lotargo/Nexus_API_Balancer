# Authentication & Authorization

## Auth Flow

1. Client presents a JWT Bearer token or admin key
2. `AuthToken` extractor validates the token
3. Claims determine role (`admin` or `client`)
4. Admin endpoints require `AdminToken` extractor (role check)

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

## KV Cache

Per-client, per-pool KV cache flag stored in `client_pools.kv_cache`. When enabled on Google pools, requests are automatically upgraded to `/v1beta/` for context caching support.
