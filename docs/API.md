# REST API

Interactive docs available at `/scalar` (via utoipa-scalar).

## Endpoints

### Proxy

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `ANY` | `/proxy/{pool_name}` | Bearer | Proxy request to named pool |
| `ANY` | `/proxy/{pool_name}/*path` | Bearer | Proxy with path suffix |
| `ANY` | `/v1/*path` | Bearer | Unified gateway — auto-routes by model |
| `ANY` | `/v1beta/*path` | Bearer | Unified gateway (Gemini compat) |

### Model Discovery

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `GET` | `/v1/models` | Bearer | List available models (OpenAI-compatible) |

### Admin

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `GET` | `/stats` | Admin | Request statistics (count, tokens, success rate) |
| `GET` | `/config` | Admin | Current configuration (secrets redacted) |
| `PATCH` | `/config` | Admin | Update server/auth config at runtime |
| `POST` | `/admin/clients` | Admin | Register a new client |
| `GET` | `/admin/keys/{pool}/{key_id}` | Admin | Export key with secret |
| `POST` | `/admin/keys/{pool}` | Admin | Import a new key |

### Public

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `POST` | `/auth/register` | None | Self-registration (if enabled) |

### MCP

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `POST` | `/mcp` | Bearer | JSON-RPC MCP endpoint |

### Misc

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `POST` | `/execute` | Bearer | Execute a named task against primary pool |
| `GET` | `/scalar` | None | Swagger UI documentation |

## Authentication

- **Bearer token** (JWT): passed via `Authorization: Bearer <token>`
- **Admin key**: passed via `X-Admin-Key` header
- **API key fallback**: `x-goog-api-key`, `x-api-key`, `api-key` headers or `key=` query param
- **Master key**: configured in `auth.master_key`, bypasses JWT validation

## Model Routing

The unified gateway (`/v1/*`) routes by:
1. **Explicit prefix**: `//provider//model_name` in the model field
2. **Model Registry**: O(1) lookup from auto-discovered models
3. **Heuristic fallback**: model name prefix matching (gpt- → openai, claude- → anthropic, etc.)
