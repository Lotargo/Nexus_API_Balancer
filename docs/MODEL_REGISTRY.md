# Dynamic Model Registry

Auto-discovers models from upstream AI providers and enables intelligent request routing.

## How It Works

1. At startup, the registry fetches `/models` from each configured pool's provider
2. Models are stored in SQLite (`provider_models` table)
3. An in-memory cache maps `model_id → [(pool_name, priority)]` for O(1) lookup
4. Cache is sorted by pool priority (higher = preferred)
5. Periodic sync every 6 hours keeps models up to date

## Model Discovery

- **OpenAI-compatible**: Parses `{ "data": [{ "id": "...", "owned_by": "..." }] }`
- **Gemini**: Parses `{ "models": [{ "name": "models/gemini-..." }] }`, strips `models/` prefix
- Custom endpoints via `models_endpoint` in pool config

## Stale Model Cleanup

- Models are marked stale before resync
- Stale models older than 24 hours are deleted

## Routing Priority

When multiple pools serve the same model, the pool with the highest `priority` value is selected. This is configured per-pool in `config.yaml`.

## Manual Override

Clients can force a specific provider with the `//provider//model_name` syntax in the model field:

```json
{ "model": "//groq//llama-3.1-8b" }
```
