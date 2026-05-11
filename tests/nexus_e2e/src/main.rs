use reqwest::Client;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio::task;
use axum::{routing::post, Json, Router, http::HeaderMap};
use std::net::SocketAddr;
use nexus_balancer::{run_server, config::AppConfig, db::Database};

const BALANCER_URL: &str = "http://127.0.0.1:3000";
const MASTER_KEY: &str = "nexus-master-key-2024";
const ADMIN_KEY: &str = "admin-secret-key-2024";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok(); // Load .env from root

    // 1. Start Mock Provider
    tokio::spawn(async {
        let app = Router::new().route("/*path", post(handle_mock_request));
        let addr = SocketAddr::from(([127, 0, 0, 1], 8085));
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        axum::serve(listener, app).await.unwrap();
    });

    // 2. Start Balancer inside the test
    tokio::spawn(async {
        let config = AppConfig::load("../../config.yaml").unwrap();
        let db = Database::new("sqlite::memory:").await.unwrap(); 
        run_server(config, db, "../../secrets").await.unwrap();
    });

    println!("--- Waiting for internal servers to start ---");
    tokio::time::sleep(Duration::from_secs(3)).await;

    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    // --- TEST 1: Concurrency (30 requests) ---
    println!("\n[TEST 1] Concurrency & Isolation");
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
                Ok(r) if r.status() == 200 => Ok(()),
                Ok(r) => Err(format!("Status {}", r.status())),
                Err(e) => Err(e.to_string()),
            }
        }));
    }

    let mut success_count = 0;
    for h in handles {
        if h.await?.is_ok() { success_count += 1; }
    }
    println!("Result: {}/30 successful in {:?}", success_count, start.elapsed());
    assert_eq!(success_count, 30, "Concurrency test failed");

    // --- TEST 2: Dynamic Key Import/Export ---
    println!("\n[TEST 2] Dynamic Key Management");
    let new_key = json!({
        "key": {
            "id": "RUST_E2E_KEY",
            "limit": 100,
            "concurrency": 2,
            "secret_name": "rust_e2e_secret",
            "secret_type": "api_key"
        },
        "secret": "sk-rust-e2e-888"
    });

    let import_resp = client.post(format!("{}/admin/keys/openai-pool", BALANCER_URL))
        .header("X-Admin-Key", ADMIN_KEY)
        .json(&new_key)
        .send()
        .await?;

    assert_eq!(import_resp.status(), 201, "Import failed: {}", import_resp.text().await?);
    println!("SUCCESS: Key imported");
    
    let mut found = false;
    for _ in 0..10 {
        let r = client.post(format!("{}/proxy/openai-pool/test", BALANCER_URL))
            .header("Authorization", format!("Bearer {}", MASTER_KEY))
            .json(&json!({}))
            .send().await?;
        let body: Value = r.json().await?;
        if body["received_headers"]["authorization"].as_str().unwrap_or("").contains("sk-rust-e2e-888") {
            found = true;
            break;
        }
    }
    assert!(found, "Key not found in rotation after import");
    println!("VERIFIED: Key active in rotation");

    // --- TEST 3: MCP ---
    println!("\n[TEST 3] MCP Interface");
    let mcp_req = json!({"jsonrpc": "2.0", "id": 1, "method": "list_pools"});
    let mcp_resp = client.post(format!("{}/mcp", BALANCER_URL))
        .header("Authorization", format!("Bearer {}", MASTER_KEY))
        .json(&mcp_req)
        .send().await?;
    
    assert_eq!(mcp_resp.status(), 200, "MCP failed: {}", mcp_resp.text().await?);
    let body: Value = mcp_resp.json().await?;
    assert!(body["result"].is_array(), "Invalid MCP response");
    println!("SUCCESS: MCP list_pools OK");

    println!("\n--- [PASSED] Pure Rust E2E Suite Completed Successfully ---");
    Ok(())
}

async fn handle_mock_request(headers: HeaderMap, Json(body): Json<Value>) -> Json<Value> {
    let mut received_headers = serde_json::Map::new();
    for (name, value) in headers.iter() {
        received_headers.insert(name.to_string(), Value::String(value.to_str().unwrap_or("").to_string()));
    }
    Json(json!({"status": "success", "received_headers": received_headers}))
}
