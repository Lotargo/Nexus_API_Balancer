use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt};
use serde_json::{json, Value};

pub async fn run_stdio_client() {
    let mcp_url = std::env::var("NEXUS_MCP_URL")
        .unwrap_or_else(|_| "http://localhost:3317/mcp".to_string());
    let api_key = std::env::var("NEXUS_API_KEY")
        .unwrap_or_else(|_| "".to_string());

    let client = reqwest::Client::new();
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    let mut reader = io::BufReader::new(stdin).lines();

    while let Ok(Some(line)) = reader.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Try to parse the request to extract ID for error responses
        let request_id = match serde_json::from_str::<Value>(trimmed) {
            Ok(v) => v.get("id").cloned().unwrap_or(Value::Null),
            Err(e) => {
                let error_resp = json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": {
                        "code": -32700,
                        "message": format!("Parse error: {}", e)
                    }
                });
                let _ = stdout.write_all(format!("{}\n", error_resp).as_bytes()).await;
                let _ = stdout.flush().await;
                continue;
            }
        };

        let response = client.post(&mcp_url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", api_key))
            .body(trimmed.to_string())
            .send()
            .await;

        match response {
            Ok(resp) => {
                match resp.text().await {
                    Ok(text) => {
                        let _ = stdout.write_all(format!("{}\n", text).as_bytes()).await;
                        let _ = stdout.flush().await;
                    }
                    Err(e) => {
                        let error_resp = json!({
                            "jsonrpc": "2.0",
                            "id": request_id,
                            "error": {
                                "code": -32000,
                                "message": format!("HTTP Adapter Read Error: {}", e)
                            }
                        });
                        let _ = stdout.write_all(format!("{}\n", error_resp).as_bytes()).await;
                        let _ = stdout.flush().await;
                    }
                }
            }
            Err(e) => {
                let error_resp = json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "error": {
                        "code": -32000,
                        "message": format!("HTTP Adapter Request Error: {}", e)
                    }
                });
                let _ = stdout.write_all(format!("{}\n", error_resp).as_bytes()).await;
                let _ = stdout.flush().await;
            }
        }
    }
}
