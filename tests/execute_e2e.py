import requests
import json
import time

def test_proxy(pool_name, path, payload):
    url = f"http://127.0.0.1:3000/proxy/{pool_name}/{path}"
    print(f"\n--- Testing Pool: {pool_name} ---")
    try:
        response = requests.post(url, json=payload)
        print(f"Status Code: {response.status_code}")
        print(f"Response: {json.dumps(response.json(), indent=2)}")
    except Exception as e:
        print(f"Error: {e}")

time.sleep(10) # Wait for balancer to start

# Test OpenAI Pool
test_proxy("openai-pool", "v1/chat/completions", {"model": "gpt-4o", "messages": [{"role": "user", "content": "Hello OpenAI"}]})

# Test Gemini Pool
test_proxy("gemini-pool", "v1beta/models/gemini-pro:generateContent", {"contents": [{"parts": [{"text": "Hello Gemini"}]}]})

# Test Anthropic Pool
test_proxy("anthropic-pool", "v1/messages", {"model": "claude-3-opus", "max_tokens": 1024, "messages": [{"role": "user", "content": "Hello Anthropic"}]})
