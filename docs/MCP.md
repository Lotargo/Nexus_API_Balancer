# Model Context Protocol (MCP)

nexus_balancer implements a JSON-RPC 2.0 MCP server for programmatic pool and key management by AI agents.

## Transport

- **Server**: JSON-RPC over HTTP at `POST /mcp`
- **Client**: stdio-to-HTTP bridge (`nexus_balancer mcp`), reads JSON-RPC from stdin, forwards to server, writes response to stdout

## Protocol

Methods are dispatched via JSON-RPC 2.0. Tool execution is wrapped under `tools/call`.

### `initialize`

Standard MCP initialization. Returns protocol version and server capabilities.

**Params**: none required

### `notifications/initialized`

Sent by the client after receiving the `initialize` result. Server returns an empty result.

### `tools/list`

List all available tools.

**Auth**: Any authenticated client

**Tools returned**:
- `list_pools` — list pools and status
- `update_description` — update pool description (admin only)
- `export_key` — export key and secret (admin only)
- `import_key` — import a new key to a pool (admin or client with pool authorization)

### `tools/call`

Execute a tool.

**Auth**: Depends on the tool (see above)

**Params**:
```json
{
  "name": "import_key",
  "arguments": {
    "pool_name": "my-pool",
    "key_cfg": {
      "id": "key-2",
      "concurrency": 2,
      "secret_type": "bearer",
      "rps_limit": 10
    },
    "secret": "sk-...",
    "provider": null,
    "kv_cache": false
  }
}
```

If the pool does not exist and `provider` is specified, the pool is auto-created.

### `resources/list`

List available resources. Returns `config://main`.

### `resources/read`

Read a resource.

**Auth**: Any authenticated client

**Params**:
```json
{
  "uri": "config://main"
}
```

Returns the full application config (secrets redacted).

## Running the stdio client

```bash
NEXUS_MCP_URL="http://localhost:3317/mcp" \
NEXUS_API_KEY="<token>" \
nexus_balancer mcp
```
