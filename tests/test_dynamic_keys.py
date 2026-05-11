import requests
import json
import time
import os

def run_test():
    base_url = "http://127.0.0.1:3000"
    admin_headers = {"X-Admin-Key": "admin-secret-key-2024"}
    
    print("\n--- 1. Testing Key Export ---")
    resp = requests.get(f"{base_url}/admin/keys/openai-pool/OPENAI_MOCK", headers=admin_headers)
    if resp.status_code == 200:
        data = resp.json()
        print(f"SUCCESS: Exported key {data['key']['id']}")
        print(f"Secret: {data['secret']}")
    else:
        print(f"FAILED: Export returned {resp.status_code}")
        return

    print("\n--- 2. Testing Key Import ---")
    new_key_payload = {
        "key": {
            "id": "DYNAMIC_KEY",
            "limit": 100,
            "concurrency": 1,
            "secret_name": "dynamic_secret_file",
            "secret_type": "api_key"
        },
        "secret": "sk-dynamic-999"
    }
    resp = requests.post(f"{base_url}/admin/keys/openai-pool", headers=admin_headers, json=new_key_payload)
    if resp.status_code == 201:
        print("SUCCESS: Key imported and injected into running pool")
    else:
        print(f"FAILED: Import returned {resp.status_code} | {resp.text}")
        return

    print("\n--- 3. Verifying Key Rotation (Proxy Test) ---")
    # We will send a few requests and look for our new secret
    found_dynamic = False
    for i in range(10):
        proxy_resp = requests.post(
            f"{base_url}/proxy/openai-pool/v1/test", 
            headers={"Authorization": "Bearer nexus-master-key-2024"},
            json={"msg": "test"}
        )
        if proxy_resp.status_code == 200:
            auth_header = proxy_resp.json().get("received_headers", {}).get("authorization", "")
            if "sk-dynamic-999" in auth_header:
                found_dynamic = True
                print(f"Req {i}: Found new key! {auth_header}")
                break
            else:
                print(f"Req {i}: Old key used: {auth_header}")
        time.sleep(0.1)

    if found_dynamic:
        print("\n✅ DYNAMIC INJECTION VERIFIED: New key is being balanced!")
    else:
        print("\n❌ VERIFICATION FAILED: New key not found in rotation")

    print("\n--- 4. Checking Persistence ---")
    if os.path.exists("secrets/dynamic_secret_file"):
        print("SUCCESS: Secret file created on disk")
    else:
        print("FAILED: Secret file missing")

    with open("config.yaml", "r") as f:
        config_content = f.read()
        if "DYNAMIC_KEY" in config_content:
            print("SUCCESS: config.yaml updated with new key")
        else:
            print("FAILED: config.yaml not updated")

if __name__ == "__main__":
    run_test()
