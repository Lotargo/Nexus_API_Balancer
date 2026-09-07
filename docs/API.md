# REST API

Interactive docs available at `/scalar` (via utoipa-scalar).

## Endpoints

### Proxy

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `ANY` | `/proxy/{pool_name}` | Bearer | Proxy request to named pool |
| `ANY` | `/proxy/{pool_name}/*path` | Bearer | Proxy with path suffix |
| `ANY` | `/v1/*path` | Bearer | Unified gateway — routes by model or capability |
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
| `POST` | `/admin/clients` | Admin | Register a new client (returns 201) |
| `GET` | `/admin/keys/{pool}/{key_id}` | Admin | Export key with secret (200) |
| `POST` | `/admin/keys/{pool}` | Bearer | Import a new key (returns 201). Non-admin clients require pool authorization |

### Public

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `POST` | `/auth/register` | None | Self-registration if enabled (returns 201, 403 if disabled) |

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
3. **Heuristic fallback**: model name prefix matching (gpt- → openai, claude- → anthropic, gemini- → gemini, etc.)


## Speech-to-Text

OpenAI-compatible STT is accepted through the unified gateway:

```http
POST /v1/audio/transcriptions
Content-Type: multipart/form-data
Authorization: Bearer <nexus-client-token>
```

Required form fields:

- `file` - audio payload
- `model` - a concrete model, a logical model alias, or `//provider//model` for explicit routing

STT requests are limited to pools declaring the `stt` capability. When no explicit provider is requested, Nexus builds an ordered candidate list and can fail over to the next provider pool on 401, 403, 404, 408, 429, or 5xx responses.

A pool may map the client model to a provider-specific model:

```yaml
capabilities: ["stt"]
capability_models:
  stt: "provider-model-id"
```
