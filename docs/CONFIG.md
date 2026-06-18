# Configuration

Configuration is loaded from `config.yaml` with optional `.env` overrides.

## config.yaml

```yaml
server:
  host: "0.0.0.0"
  port: 3317
  cors_allowed_origin: "http://localhost:3317"

auth:
  enabled: true
  public_registration: false
  master_key: null
  admin_key: null
  secret: "<jwt-signing-secret>"
  issuer: "nexus"
  audience: "nexus-api"

pools:
  - name: "my-pool"
    description: "OpenAI pool"
    provider: "openai"
    target_url: "https://api.openai.com/v1"
    capacity: 100
    priority: 0
    models_endpoint: null
    skip_model_sync: false
    keys:
      - id: "key-1"
        secret_name: "openai_api_key.txt"
        secret_type: "bearer"
        concurrency: 3
        rps_limit: 10
        rpd_limit: 10000
        tpm_limit: 100000
        tpd_limit: 10000000
        max_request_tokens: 128000
        cooldown_on_limit: false
```

## Environment Variables

| Variable | Overrides | Default |
|----------|-----------|---------|
| `HOST` | `server.host` | — |
| `PORT` | `server.port` | — |
| `CORS_ALLOWED_ORIGIN` | `server.cors_allowed_origin` | `http://localhost:3317` |
| `DATABASE_URL` | DB path | `sqlite:nexus.db` |
| `ADMIN_API_KEY` | `auth.admin_key` | `admin-secret-key-2026` |

## Secret Storage

API keys are stored in a `secrets/` directory (configurable path passed to `run_server`). Format:

- One key per line (multiple keys per file supported)
- Client-isolated when imported via MCP: `secrets/{client_id}/{provider}_api_key.txt`
- BOM and whitespace are trimmed automatically

## Supported Providers

| Provider | Default Target URL |
|----------|--------------------|
| openai | `https://api.openai.com/v1` |
| gemini / google | `https://generativelanguage.googleapis.com` |
| gemini-openai | `https://generativelanguage.googleapis.com/v1beta/openai` |
| grok / xai | `https://api.x.ai/v1` |
| groq | `https://api.groq.com/openai/v1` |
| cerebras | `https://api.cerebras.ai/v1` |
| sambanova | `https://api.sambanova.ai/v1` |
| cohere | `https://api.cohere.com/v2` |
| mistral | `https://api.mistral.ai/v1` |
| deepseek | `https://api.deepseek.com` |
| anthropic / claude | `https://api.anthropic.com/v1` |

## Key Capacity Calculation

At startup, each pool's capacity is automatically raised to `max(configured_capacity, sum(secrets_per_key * concurrency_for_each_key))` to prevent capacity exceeded errors.
