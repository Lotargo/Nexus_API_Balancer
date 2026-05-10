import requests
import sys

BASE_URL = "http://127.0.0.1:3000"
ADMIN_KEY = "super-secret-admin-key-change-me" # Default from .env.example

def test_admin_access():
    print("Testing Admin Access...")
    # 1. Access /config without key
    resp = requests.get(f"{BASE_URL}/config")
    print(f"  GET /config (no key): {resp.status_code} (Expected 401)")
    
    # 2. Access /config with wrong key
    resp = requests.get(f"{BASE_URL}/config", headers={"X-Admin-Key": "wrong"})
    print(f"  GET /config (wrong key): {resp.status_code} (Expected 401/403)")
    
    # 3. Access /config with correct key
    resp = requests.get(f"{BASE_URL}/config", headers={"X-Admin-Key": ADMIN_KEY})
    print(f"  GET /config (correct key): {resp.status_code} (Expected 200)")

def test_isolation_and_masking():
    print("\nTesting Isolation and Masking...")
    # This requires a running DB with specific data.
    # We expect that if we are not admin, secrets are redacted.
    # resp = requests.get(f"{BASE_URL}/config", headers={"Authorization": "Bearer some-client-token"})
    print("  (Manual check) Verify that secrets in /config are [REDACTED]")

if __name__ == "__main__":
    try:
        test_admin_access()
        test_isolation_and_masking()
    except Exception as e:
        print(f"Error: {e}")
        print("Is the server running?")
