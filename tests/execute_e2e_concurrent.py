import requests
import json
import time
from concurrent.futures import ThreadPoolExecutor

def test_proxy(pool_name, path, payload, request_id):
    url = f"http://127.0.0.1:3000/proxy/{pool_name}/{path}"
    headers = {"Authorization": "Bearer nexus-master-key-2024"}
    try:
        start_time = time.time()
        response = requests.post(url, json=payload, headers=headers)
        latency = (time.time() - start_time) * 1000
        
        # Check if the received key matches the pool (security check)
        received_auth = response.json().get("received_headers", {}).get("authorization", "")
        received_google_key = response.json().get("received_headers", {}).get("x-goog-api-key", "")
        received_anthropic_key = response.json().get("received_headers", {}).get("x-api-key", "")
        
        status = "OK"
        if "openai" in pool_name and "openai" not in received_auth: status = "ERROR: LEAK"
        if "gemini" in pool_name and "gemini" not in received_google_key: status = "ERROR: LEAK"
        if "anthropic" in pool_name and "anthropic" not in received_anthropic_key: status = "ERROR: LEAK"

        if request_id == 0:
            print(f"DEBUG: Sample Response: {json.dumps(response.json(), indent=2)}")

        print(f"Req {request_id:02d} | Pool: {pool_name:15} | Status: {response.status_code} | Latency: {latency:6.1f}ms | {status}")
        return status == "OK"
    except Exception as e:
        print(f"Req {request_id:02d} | Pool: {pool_name:15} | Error: {e}")
        return False

# List of concurrent requests
requests_to_send = []
for i in range(10):
    requests_to_send.append(("openai-pool", "v1/chat", {"id": i}))
    requests_to_send.append(("gemini-pool", "v1/gen", {"id": i}))
    requests_to_send.append(("anthropic-pool", "v1/msg", {"id": i}))

print(f"Starting {len(requests_to_send)} concurrent requests...")
start_all = time.time()

with ThreadPoolExecutor(max_workers=30) as executor:
    results = list(executor.map(lambda x, idx: test_proxy(x[0], x[1], x[2], idx), 
                               requests_to_send, range(len(requests_to_send))))

total_time = time.time() - start_all
success_count = sum(1 for r in results if r)

print(f"\n--- Concurrency Test Summary ---")
print(f"Total Requests: {len(requests_to_send)}")
print(f"Successful:     {success_count}")
print(f"Total Time:       {total_time:.2f}s")
print(f"Requests/sec:   {len(requests_to_send)/total_time:.2f}")

if success_count == len(requests_to_send):
    print("\nSUCCESS: ISOLATION VERIFIED: No leaks or cross-contamination detected.")
else:
    print("\nFAILURE: TEST FAILED: Some requests failed or leaked data!")
