# Model Context Protocol (MCP)

nexus_balancer implements a JSON-RPC 2.0 MCP server for programmatic pool and key management by AI agents.

## Transport

- **Server**: JSON-RPC over HTTP at `POST /mcp`
- **Client**: stdio-to-HTTP bridge (`nexus_balancer mcp`), reads JSON-RPC from stdin, forwards to server, writes response to stdout

## Methods

### `list_pools`

List all pools with descriptions, key counts, and capacity.

**Auth**: Any authenticated client (non-admin sees only their allowed pools)

### `update_description`

Update a pool's description.

**Auth**: Admin only

**Params**:
```json
{ "pool_name": "my-pool", "description": "New description" }
```

### `export_key`

Export a key with its secret by pool name and key ID.

**Auth**: Admin only

**Params**:
```json
{ "pool_name": "my-pool", "key_id": "key-1" }
```

### `import_key`

Import a new key into a pool. Validates the key against the provider's API before saving.

**Auth**: Admin only

**Params**:
```json
{
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
```

If the pool does not exist and `provider` is specified, the pool is auto-created.

## Resources

### `config://main`

Returns the full application config (secrets redacted).

**Auth**: Any authenticated client

**Example**:
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "resources/read",
  "params": { "uri": "config://main" }
}
```

## Running the stdio client

```bash
NEXUS_MCP_URL="http://localhost:3317/mcp" \
NEXUS_API_KEY="<token>" \
nexus_balancer mcp
```
