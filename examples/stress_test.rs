use tokio::time::{Instant};
use std::sync::Arc;
use tokio::sync::Mutex;
use reqwest::Client;
use serde_json::json;
use jsonwebtoken::{encode, Header, EncodingKey};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Default)]
struct Metrics {
    success: u32,
    rate_limited: u32,
    errors: u32,
    latencies: Vec<u128>,
}

fn gen_token(secret: &str) -> String {
    let my_claims = json!({
        "sub": "stress-test",
        "exp": (SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 3600) as usize,
        "iss": "lotargo-balancer",
        "aud": "api-clients"
    });

    encode(
        &Header::default(),
        &my_claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    ).unwrap()
}

#[tokio::main]
async fn main() {
    let url = "http://127.0.0.1:8080/execute";
    let secret = "my-very-secure-shared-secret-for-jwt";
    let token = gen_token(secret);
    
    let client = Client::new();
    let metrics = Arc::new(Mutex::new(Metrics::default()));
    
    let iterations = 500;
    let concurrency = 30;
    
    println!("Starting stress test: {} requests, concurrency {}...", iterations, concurrency);
    
    let start_time = Instant::now();
    let mut tasks = vec![];

    for i in 0..iterations {
        let client = client.clone();
        let token = token.clone();
        let metrics = metrics.clone();
        
        tasks.push(tokio::spawn(async move {
            let req_start = Instant::now();
            let res = client.post(url)
                .header("Authorization", format!("Bearer {}", token))
                .json(&json!({"task_name": format!("task-{}", i)}))
                .send()
                .await;

            let latency = req_start.elapsed().as_millis();
            let mut m = metrics.lock().await;
            m.latencies.push(latency);

            match res {
                Ok(response) => {
                    let body: serde_json::Value = response.json().await.unwrap_or_default();
                    if body["status"] == "success" {
                        m.success += 1;
                    } else if body["status"] == "error" && body["message"].as_str().unwrap_or("").contains("limit") {
                        m.rate_limited += 1;
                    } else {
                        m.errors += 1;
                    }
                }
                Err(_) => m.errors += 1,
            }
        }));

        if tasks.len() >= concurrency {
            futures::future::join_all(tasks.drain(..)).await;
        }
    }
    
    futures::future::join_all(tasks).await;
    let total_duration = start_time.elapsed();
    
    let m = metrics.lock().await;
    let avg_latency = m.latencies.iter().sum::<u128>() as f64 / m.latencies.len() as f64;
    let mut sorted_latencies = m.latencies.clone();
    sorted_latencies.sort();
    let p95 = if !sorted_latencies.is_empty() { sorted_latencies[(sorted_latencies.len() as f64 * 0.95) as usize] } else { 0 };

    println!("\n--- Load Balancer Stress Test Results ---");
    println!("Total Requests:  {}", iterations);
    println!("Duration:        {:.2}s", total_duration.as_secs_f64());
    println!("Throughput:      {:.2} req/sec", iterations as f64 / total_duration.as_secs_f64());
    println!("Success Status:  {} (authorized)", m.success);
    println!("Rate Limited:    {} (blocked)", m.rate_limited);
    println!("Error Rate:      {:.1}%", (m.errors as f64 / iterations as f64) * 100.0);
    println!("Avg Latency:     {:.2} ms", avg_latency);
    println!("P95 Latency:     {} ms", p95);
    println!("-----------------------------------------");
}
