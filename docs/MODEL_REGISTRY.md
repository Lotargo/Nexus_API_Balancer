# Dynamic Model Registry

Auto-discovers models from upstream AI providers and enables intelligent request routing.

## How It Works

1. At startup, the registry fetches `/models` from each configured pool's provider
2. Models are stored in SQLite (`provider_models` table)
3. An in-memory cache maps `model_id → [(pool_name, priority)]` for O(1) lookup
4. Cache is sorted by pool priority (higher = preferred)
5. Periodic sync every 6 hours keeps models up to date

## Model Discovery

Discovery is per-pool. Each pool's `target_url` is queried at `{target_url}/models` (or `models_endpoint` if set).

- **OpenAI-compatible**: Parses `{ "data": [{ "id": "...", "owned_by": "..." }] }`
- **Gemini (native)**: Parses `{ "models": [{ "name": "models/gemini-..." }] }`, strips `models/` prefix
- **Gemini OpenAI-compatible**: Queries `{target_url}/models` — returns OpenAI-compatible format
- **Custom endpoints**: Override per-pool via `models_endpoint` config field

### Provider-specific URL behavior

| Provider | Discovery URL | Response Format |
|----------|--------------|-----------------|
| openai | `https://api.openai.com/v1/models` | OpenAI-compat |
| gemini (native) | `https://generativelanguage.googleapis.com/v1beta/models` | Gemini native |
| gemini-openai | `https://generativelanguage.googleapis.com/v1beta/openai/models` | OpenAI-compat |
| anthropic | `https://api.anthropic.com/v1/models` | OpenAI-compat |
| groq | `https://api.groq.com/openai/v1/models` | OpenAI-compat |

## Discovery Failures

If a provider's API is unavailable or the key is invalid, the sync skips that pool with a warning — the server does not crash. Models from previous syncs remain in the DB until stale cleanup (>24h).

## Stale Model Cleanup

- Models are marked `is_stale = 1` before resync
- Stale models older than 24 hours are deleted

## Routing Priority

When multiple pools serve the same model, the pool with the highest `priority` value is selected. Configurable per-pool.

## Skip Sync

Per-pool `skip_model_sync: true` excludes the pool from auto-discovery entirely.

## Models Endpoint Fallback

If `models_endpoint` is not set, defaults to `/models` appended to `target_url`.

## Manual Override

Clients can force a specific provider with the `//provider//model_name` syntax in the model field:

```json
{ "model": "//groq//llama-3.1-8b" }
```

## Heuristic Fallback

If the model is not found in the registry, prefix-based matching is used:

| Prefix | Provider |
|--------|----------|
| `gpt-`, `o1-`, `text-davinci` | openai |
| `claude-` | anthropic / claude |
| `gemini-` | google / gemini |
| `deepseek-` | deepseek |
| `mistral-`, `codestral-`, `pixtral-`, `ministral-`, `open-mixtral-` | mistral |
