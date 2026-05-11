# Nexus API Balancer

Rust-based proxy and key balancer for AI providers. It routes requests through named pools, injects provider credentials, enforces per-key limits, and logs usage to SQLite.

## What it does

- Proxies OpenAI-compatible, Anthropic, and Gemini requests.
- Supports both regular JSON responses and upstream `text/event-stream` passthrough.
- Preserves request path and query string, which is required for Gemini SSE (`?alt=sse`).
- Tracks request/token usage per key and writes aggregate stats to SQLite.
- Supports JWT auth, a shared master key, admin endpoints, and MCP JSON-RPC helpers.

## Quick start

1. Create local config and secrets:

```bash
cp .env.example .env
cp config.yaml.example config.yaml
mkdir -p secrets
```

2. Put provider keys into files inside `secrets/`. Example:

```bash
echo "your-gemini-key" > secrets/gemini_key_1
echo "your-openai-key" > secrets/openai_key_1
```

3. Start the server:

```bash
cargo run
```

The server listens on the `server.port` from `config.yaml`. The local project config in this repo uses `127.0.0.1:3000`.

## Proxy usage

Proxy endpoint:

```text
/proxy/:pool_name/*path
```

Headers are forwarded except client auth headers, and the balancer injects the upstream provider credential automatically.

Example OpenAI-compatible call:

```bash
curl -X POST http://127.0.0.1:3000/proxy/openai-pool/v1/chat/completions \
  -H "Authorization: Bearer nexus-master-key-2024" \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hello"}]}'
```

Example Gemini SSE call:

```bash
curl -N -X POST "http://127.0.0.1:3000/proxy/gemini-pool/models/gemini-flash-lite-latest:streamGenerateContent?alt=sse" \
  -H "Authorization: Bearer nexus-master-key-2024" \
  -H "Content-Type: application/json" \
  -d '{"contents":[{"parts":[{"text":"Say hello in one sentence"}]}]}'
```

For Gemini non-streaming requests, use:

```text
/proxy/gemini-pool/models/<model>:generateContent
```

For Gemini SSE, use:

```text
/proxy/gemini-pool/models/<model>:streamGenerateContent?alt=sse
```

## Configuration notes

- `target_url` should be a provider base URL, not a hardcoded single method URL, when you want multiple endpoints like Gemini `generateContent` and `streamGenerateContent`.
- The shared master key in `config.yaml` is intentionally supported by the architecture and can be used for trusted internal clients.
- Real provider secrets should stay in `secrets/` or environment variables used by test/setup scripts, not in tracked source files.

See [config.yaml.example](/F:/Кейс/Nexus_API_Balancer/config.yaml.example) for the current schema.

## Admin and observability

- `GET /stats` returns aggregate request/token stats.
- `GET /config` and `PATCH /config` expose runtime config for admins.
- `POST /admin/clients` creates JWT-bearing clients.
- `GET /admin/keys/:pool/:id` and `POST /admin/keys/:pool` export/import provider keys.
- `POST /mcp` exposes MCP-style JSON-RPC helpers for pool discovery and key management.

Admin auth uses `X-Admin-Key`, loaded from `ADMIN_API_KEY` in `.env`.

## Testing

Project checks:

```bash
cargo check
cd tests/nexus_e2e && cargo check
```

End-to-end suite:

```bash
cd tests/nexus_e2e
GEMINI_REAL_API_KEY=your-real-key cargo run
```

The E2E suite starts an internal mock provider, boots the balancer, validates concurrency and rate limiting, performs a real Gemini `generateContent` request, and then verifies Gemini SSE via `streamGenerateContent?alt=sse`.

## License

Apache-2.0. See [LICENSE](/F:/Кейс/Nexus_API_Balancer/LICENSE).
