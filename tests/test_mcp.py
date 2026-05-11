import requests
import json
import time

def mcp_request(method, params=None):
    url = "http://127.0.0.1:3000/mcp"
    headers = {
        "Authorization": "Bearer nexus-master-key-2024",
        "Content-Type": "application/json"
    }
    payload = {
        "jsonrpc": "2.0",
        "id": int(time.time()),
        "method": method,
        "params": params
    }
    resp = requests.post(url, json=payload, headers=headers)
    return resp.json()

def run_test():
    print("--- Testing MCP: list_pools ---")
    res = mcp_request("list_pools")
    print(json.dumps(res, indent=2))
    if res.get("result"):
        print("SUCCESS: list_pools returned pools")
    else:
        print("FAILED: list_pools failed")
        return

    print("\n--- Testing MCP: export_key ---")
    res = mcp_request("export_key", {"pool_name": "openai-pool", "key_id": "OPENAI_MOCK"})
    print(json.dumps(res, indent=2))
    if res.get("result") and "secret" in res["result"]:
        print("SUCCESS: export_key via MCP works")
    else:
        print("FAILED: export_key via MCP failed")
        return

    print("\n--- Testing MCP: import_key ---")
    import_params = {
        "pool_name": "openai-pool",
        "key_cfg": {
            "id": "MCP_DYNAMIC_KEY",
            "limit": 500,
            "concurrency": 3,
            "secret_name": "mcp_secret_file",
            "secret_type": "api_key"
        },
        "secret": "sk-mcp-test-789"
    }
    res = mcp_request("import_key", import_params)
    print(json.dumps(res, indent=2))
    if res.get("result") and "imported successfully" in res["result"]:
        print("SUCCESS: import_key via MCP works")
    else:
        print("FAILED: import_key via MCP failed")

if __name__ == "__main__":
    run_test()
