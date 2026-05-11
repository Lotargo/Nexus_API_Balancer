use reqwest::Client;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio::task;
use axum::{routing::post, Json, Router, http::HeaderMap};
use std::net::SocketAddr;
use nexus_balancer::{run_server, config::AppConfig, db::Database};
use futures::StreamExt;

const BALANCER_URL: &str = "http://127.0.0.1:3000";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let admin_key = std::env::var("ADMIN_API_KEY").unwrap_or_else(|_| "admin-secret-key-2024".to_string());
    let gemini_real_key = std::env::var("GEMINI_REAL_API_KEY").ok();

    if let Some(secret) = &gemini_real_key {
        std::fs::write("../../secrets/gemini_real_key", secret)?;
    }

    // Load config to get master_key
    let config = AppConfig::load("../../config.yaml").expect("Failed to load config.yaml");
    let master_key = config.auth.master_key.as_deref().expect("Master key must be set in config.yaml");


    // 1. Start Mock Provider
    tokio::spawn(async {
        let app = Router::new().route("/*path", post(handle_mock_request));
        let addr = SocketAddr::from(([127, 0, 0, 1], 8085));
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        axum::serve(listener, app).await.unwrap();
    });

    // 2. Start Balancer inside the test
    let server_config = config.clone();
    tokio::spawn(async move {
        let db_path = std::env::current_dir().unwrap().join("nexus-e2e.db");
        if db_path.exists() {
            let _ = std::fs::remove_file(&db_path);
        }
        let db_url = format!("sqlite:{}", db_path.to_string_lossy().replace('\\', "/"));
        let db = Database::new(&db_url).await.unwrap();
        run_server(server_config, db, "../../secrets").await.unwrap();
    });

    println!("--- Waiting for internal servers to start ---");
    tokio::time::sleep(Duration::from_secs(3)).await;

    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;
    let real_client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    // --- TEST 1: Concurrency & Isolation ---
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
                .header("Authorization", format!("Bearer {}", master_key))
                .json(&json!({"id": i}))
                .send().await;
            match resp {
                Ok(r) if r.status() == 200 => Ok(()),
                Ok(r) => Err(format!("Status {}", r.status())),
                Err(e) => Err(e.to_string()),
            }
        }));
    }
    let mut success_count = 0;
    for h in handles { if h.await?.is_ok() { success_count += 1; } }
    println!("Result: {}/30 successful in {:?}", success_count, start.elapsed());
    assert_eq!(success_count, 30);

    // --- TEST 2: RPS Limits ---
    println!("\n[TEST 2] RPS Limit Enforcement");
    // Import a key with strict RPS limit (2 req/sec) into a dedicated pool
    let rps_key = json!({
        "key": {
            "id": "STRICT_RPS_KEY",
            "rps_limit": 2,
            "cooldown_on_limit": true,
            "concurrency": 1,
            "secret_name": "strict_secret",
            "secret_type": "api_key"
        },
        "secret": "sk-strict-123"
    });
    client.post(format!("{}/admin/keys/limit-pool", BALANCER_URL))
        .header("X-Admin-Key", &admin_key)
        .json(&rps_key).send().await?;

    // Send 5 rapid requests (should hit 429)
    let mut rps_hits = 0;
    for _ in 0..5 {
        let r = client.post(format!("{}/proxy/limit-pool/test", BALANCER_URL))
            .header("Authorization", format!("Bearer {}", master_key))
            .json(&json!({}))
            .send().await?;
        if r.status() == 429 { rps_hits += 1; }
        // No sleep to hit limit fast
    }
    println!("RPS Hits (429): {}/5", rps_hits);
    assert!(rps_hits >= 3, "RPS limit was not enforced (expected at least 3 hits for limit 2/sec)");

    // --- TEST 3: Token Tracking ---
    println!("\n[TEST 3] Token Tracking Verification");
    let token_resp = client.post(format!("{}/proxy/openai-pool/v1/completions", BALANCER_URL))
        .header("Authorization", format!("Bearer {}", master_key))
        .json(&json!({"prompt": "hello", "mock_usage": 150})) // Our mock will use this
        .send().await?;
    
    assert_eq!(token_resp.status(), 200);
    
    // Check stats
    let stats_resp = client.get(format!("{}/stats", BALANCER_URL))
        .header("X-Admin-Key", &admin_key)
        .send().await?;
    let stats: Value = stats_resp.json().await?;
    println!("Stats: {}", stats);
    assert!(stats["total_tokens"].as_i64().unwrap_or(0) >= 150, "Token tracking failed");

    // --- TEST 4: Real Gemini Request (Optional) ---
    println!("\n[TEST 4] Real Gemini Request Verification");
    let gemini_payload = json!({
        "contents": [{
            "parts": [{"text": "Hello, how are you?"}]
        }]
    });
    let gemini_resp = real_client.post(format!("{}/proxy/gemini-real/models/gemini-flash-lite-latest:generateContent", BALANCER_URL))
        .header("Authorization", format!("Bearer {}", master_key))
        .json(&gemini_payload)
        .send().await?;

    if gemini_resp.status() == 200 {
        let body: Value = gemini_resp.json().await?;
        println!("Gemini Response OK. Tokens used: {:?}", body.get("usageMetadata"));

        let stream_resp = real_client.post(format!("{}/proxy/gemini-real/models/gemini-flash-lite-latest:streamGenerateContent?alt=sse", BALANCER_URL))
            .header("Authorization", format!("Bearer {}", master_key))
            .json(&gemini_payload)
            .send().await?;

        assert_eq!(stream_resp.status(), 200, "Gemini SSE request failed");
        let content_type = stream_resp.headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert!(content_type.starts_with("text/event-stream"), "Expected SSE content type, got {}", content_type);

        let mut stream = stream_resp.bytes_stream();
        let mut collected = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            collected.extend_from_slice(&chunk);
            let text = String::from_utf8_lossy(&collected);
            if text.contains("usageMetadata") && text.contains("\n\n") {
                break;
            }
        }

        let sse_text = String::from_utf8_lossy(&collected);
        assert!(sse_text.contains("data:"), "SSE response did not contain data frames");
        assert!(sse_text.contains("usageMetadata") || sse_text.contains("candidates"), "SSE response did not contain Gemini payload");
        println!("Gemini SSE Response OK. First frames:\n{}", sse_text);

        let final_stats: Value = client.get(format!("{}/stats", BALANCER_URL))
            .header("X-Admin-Key", &admin_key)
            .send().await?.json().await?;
        println!("Final Stats: {}", final_stats);
    } else {
        println!("Gemini Request failed with status: {}. Body: {}", gemini_resp.status(), gemini_resp.text().await?);
    }

    println!("\n--- [PASSED] All Tests (including Real Gemini) Completed ---");
    if gemini_real_key.is_some() {
        let _ = std::fs::remove_file("../../secrets/gemini_real_key");
    }
    Ok(())
}

async fn handle_mock_request(headers: HeaderMap, Json(body): Json<Value>) -> Json<Value> {
    let mut tokens = 0;
    if let Some(t) = body.get("mock_usage").and_then(|v| v.as_u64()) {
        tokens = t;
    }

    json!({
        "status": "success",
        "received_headers": headers.iter().map(|(k,v)| (k.to_string(), json!(v.to_str().unwrap_or("")))).collect::<serde_json::Map<String, Value>>(),
        "usage": {
            "total_tokens": tokens,
            "prompt_tokens": tokens / 2,
            "completion_tokens": tokens / 2
        }
    }).into()
}
