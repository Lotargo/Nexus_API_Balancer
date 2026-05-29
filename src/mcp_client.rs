use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt};
use serde_json::{json, Value};

pub async fn run_stdio_client() {
    let mcp_url = std::env::var("NEXUS_MCP_URL")
        .unwrap_or_else(|_| "http://localhost:3317/mcp".to_string());

    // Auth token
    let api_key = std::env::var("NEXUS_API_KEY")
        .unwrap_or_else(|_| "".to_string());

    let client = reqwest::Client::new();
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();

    let mut reader = io::BufReader::new(stdin).lines();

    while let Ok(Some(line)) = reader.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Try to parse the request to extract ID for error responses
        let parsed: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
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

        let request_id = parsed.get("id").cloned();
        let is_notification = request_id.is_none();

        let mut req_builder = client.post(&mcp_url)
            .header("Content-Type", "application/json");

        if !api_key.is_empty() {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", api_key));
        }

        let response = req_builder.body(trimmed.to_string()).send().await;

        match response {
            Ok(resp) => {
                match resp.text().await {
                    Ok(text) => {
                        // If it's a notification, the server might return an empty response, which is valid.
                        if is_notification {
                            if !text.trim().is_empty() {
                                // Sometimes server responds to notifications with an empty json object
                                let _ = stdout.write_all(format!("{}\n", text).as_bytes()).await;
                                let _ = stdout.flush().await;
                            }
                        } else {
                            let _ = stdout.write_all(format!("{}\n", text).as_bytes()).await;
                            let _ = stdout.flush().await;
                        }
                    }
                    Err(e) => {
                        if !is_notification {
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
                        } else {
                            let _ = stderr.write_all(format!("Error reading notification response: {}\n", e).as_bytes()).await;
                        }
                    }
                }
            }
            Err(e) => {
                if !is_notification {
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
                } else {
                    let _ = stderr.write_all(format!("Error sending notification: {}\n", e).as_bytes()).await;
                }
            }
        }
    }

    // EOF means parent process disconnected
    let _ = stderr.write_all(b"EOF received, shutting down MCP client.\n").await;
}
