use reqwest::Client;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio::task;
use axum::{routing::post, Json, Router, http::HeaderMap};
use std::net::SocketAddr;

const BALANCER_URL: &str = "http://127.0.0.1:3000";
const MOCK_URL: &str = "http://127.0.0.1:8085";
const MASTER_KEY: &str = "nexus-master-key-2024";
const ADMIN_KEY: &str = "admin-secret-key-2024";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Start Mock Provider in background
    tokio::spawn(async {
        let app = Router::new().route("/*path", post(handle_mock_request));
        let addr = SocketAddr::from(([127, 0, 0, 1], 8085));
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        axum::serve(listener, app).await.unwrap();
    });

    println!("--- Waiting for servers to be ready ---");
    tokio::time::sleep(Duration::from_secs(2)).await;

    let client = Client::new();

    // --- TEST 1: Concurrency & Isolation ---
    println!("\n[TEST 1] Concurrency & Isolation (30 requests)");
    let start = Instant::now();
    let mut handles = vec![];

    for i in 0..30 {
        let pool = if i % 3 == 0 { "openai-pool" } 
                   else if i % 3 == 1 { "gemini-pool" } 
                   else { "anthropic-pool" };
        
        let c = client.clone();
        handles.push(task::spawn(async move {
            let resp = c.post(format!("{}/proxy/{}/v1/test", BALANCER_URL, pool))
                .header("Authorization", format!("Bearer {}", MASTER_KEY))
                .json(&json!({"id": i}))
                .send()
                .await;
            
            match resp {
                Ok(r) if r.status() == 200 => {
                    let body: Value = r.json().await.unwrap();
                    let auth = body["received_headers"]["authorization"].as_str().unwrap_or("");
                    // Basic check: verify provider specific markers if needed
                    if pool == "openai-pool" && !auth.contains("Bearer") { return Err("OpenAI missing Bearer"); }
                    Ok(())
                }
                Ok(r) => Err("Non-200 status"),
                Err(_) => Err("Request failed"),
            }
        }));
    }

    let mut success = 0;
    for h in handles {
        if h.await?.is_ok() { success += 1; }
    }
    println!("Result: {}/30 successful in {:?}", success, start.elapsed());

    // --- TEST 2: Dynamic Key Export/Import ---
    println!("\n[TEST 2] Dynamic Key Management");
    // Export
    let export_resp = client.get(format!("{}/admin/keys/openai-pool/OPENAI_MOCK", BALANCER_URL))
        .header("X-Admin-Key", ADMIN_KEY)
        .send()
        .await?;
    
    if export_resp.status() == 200 {
        println!("SUCCESS: Key exported");
    } else {
        println!("FAILED: Key export returned {}", export_resp.status());
    }

    // Import
    let new_key = json!({
        "key": {
            "id": "RUST_DYNAMIC_KEY",
            "limit": 100,
            "concurrency": 2,
            "secret_name": "rust_secret_file",
            "secret_type": "api_key"
        },
        "secret": "sk-rust-dynamic-777"
    });
    let import_resp = client.post(format!("{}/admin/keys/openai-pool", BALANCER_URL))
        .header("X-Admin-Key", ADMIN_KEY)
        .json(&new_key)
        .send()
        .await?;

    if import_resp.status() == 201 {
        println!("SUCCESS: Key imported dynamically");
        
        // Verify rotation
        let mut found = false;
        for _ in 0..10 {
            let r = client.post(format!("{}/proxy/openai-pool/test", BALANCER_URL))
                .header("Authorization", format!("Bearer {}", MASTER_KEY))
                .json(&json!({}))
                .send().await?;
            let body: Value = r.json().await?;
            if body["received_headers"]["authorization"].as_str().unwrap_or("").contains("sk-rust-dynamic-777") {
                found = true;
                break;
            }
        }
        if found { println!("VERIFIED: New key is active in pool"); }
        else { println!("FAILED: New key not found in rotation"); }
    }

    // --- TEST 3: MCP JSON-RPC ---
    println!("\n[TEST 3] MCP Interface");
    let mcp_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "list_pools"
    });
    let mcp_resp = client.post(format!("{}/mcp", BALANCER_URL))
        .header("Authorization", format!("Bearer {}", MASTER_KEY))
        .json(&mcp_req)
        .send()
        .await?;
    
    if mcp_resp.status() == 200 {
        let body: Value = mcp_resp.json().await?;
        if body["result"].is_array() {
            println!("SUCCESS: MCP list_pools returned {} pools", body["result"].as_array().unwrap().len());
        }
    }

    println!("\n--- All Rust E2E Tests Completed ---");
    Ok(())
}

async fn handle_mock_request(headers: HeaderMap, Json(body): Json<Value>) -> Json<Value> {
    let mut received_headers = serde_json::Map::new();
    for (name, value) in headers.iter() {
        received_headers.insert(name.to_string(), Value::String(value.to_str().unwrap_or("").to_string()));
    }

    Json(json!({
        "status": "success",
        "received_body": body,
        "received_headers": received_headers
    }))
}
