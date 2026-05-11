use serde_json::Value;
use tiktoken_rs::o200k_base_singleton;

pub fn count_tokens(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }

    let bpe = o200k_base_singleton();
    let token_count = bpe.lock().encode_with_special_tokens(text).len();
    token_count
}

pub fn estimate_request_tokens(json: &Value) -> usize {
    let mut segments = Vec::new();
    collect_request_segments(json, &mut segments);
    if segments.is_empty() {
        collect_all_string_segments(json, &mut segments);
    }

    segments.into_iter().map(|segment| count_tokens(&segment)).sum()
}

pub fn estimate_response_tokens(json: &Value) -> usize {
    let mut segments = Vec::new();
    collect_response_segments(json, &mut segments);
    if segments.is_empty() {
        collect_all_string_segments(json, &mut segments);
    }

    segments.into_iter().map(|segment| count_tokens(&segment)).sum()
}

fn collect_request_segments(value: &Value, segments: &mut Vec<String>) {
    match value {
        Value::String(text) => push_text(segments, text),
        Value::Array(items) => {
            for item in items {
                collect_request_segments(item, segments);
            }
        }
        Value::Object(map) => {
            for key in ["prompt", "input", "text", "instructions", "system"] {
                if let Some(field) = map.get(key) {
                    collect_textish(field, segments);
                }
            }

            for key in ["messages", "contents", "parts", "content", "system_instruction", "systemInstruction"] {
                if let Some(field) = map.get(key) {
                    collect_textish(field, segments);
                }
            }
        }
        _ => {}
    }
}

fn collect_response_segments(value: &Value, segments: &mut Vec<String>) {
    match value {
        Value::String(text) => push_text(segments, text),
        Value::Array(items) => {
            for item in items {
                collect_response_segments(item, segments);
            }
        }
        Value::Object(map) => {
            for key in ["text", "content", "message", "output_text", "output", "parts", "candidates", "choices"] {
                if let Some(field) = map.get(key) {
                    collect_textish(field, segments);
                }
            }
        }
        _ => {}
    }
}

fn collect_textish(value: &Value, segments: &mut Vec<String>) {
    match value {
        Value::String(text) => push_text(segments, text),
        Value::Array(items) => {
            for item in items {
                collect_textish(item, segments);
            }
        }
        Value::Object(map) => {
            for key in ["text", "content", "parts", "message", "messages", "contents", "input", "prompt"] {
                if let Some(field) = map.get(key) {
                    collect_textish(field, segments);
                }
            }
        }
        _ => {}
    }
}

fn collect_all_string_segments(value: &Value, segments: &mut Vec<String>) {
    match value {
        Value::String(text) => push_text(segments, text),
        Value::Array(items) => {
            for item in items {
                collect_all_string_segments(item, segments);
            }
        }
        Value::Object(map) => {
            for field in map.values() {
                collect_all_string_segments(field, segments);
            }
        }
        _ => {}
    }
}

pub async fn verify_key(client: &reqwest::Client, provider: &str, target_url: &str, secret: &str) -> Result<(), String> {
    println!(" [DEBUG] Verifying key for provider: {} at {}", provider, target_url);

    let url = match provider.to_lowercase().as_str() {
        "gemini" | "google" => format!("{}/v1beta/models?key={}", target_url.trim_end_matches('/'), secret),
        "cohere" => format!("{}/models", target_url.trim_end_matches('/')),
        _ => {
            // Default OpenAI-compatible check
            if target_url.ends_with("/v1") {
                format!("{}/models", target_url.trim_end_matches('/'))
            } else {
                format!("{}/v1/models", target_url.trim_end_matches('/'))
            }
        }
    };

    let mut request = client.get(&url);
    
    // Add auth headers (except for Gemini where it's in the URL)
    if !matches!(provider.to_lowercase().as_str(), "gemini" | "google") {
        request = request.header("Authorization", format!("Bearer {}", secret));
    }

    let resp = request.send().await
        .map_err(|e| format!("Network error during verification: {}", e))?;

    if resp.status().is_success() {
        Ok(())
    } else {
        let status = resp.status();
        let error_text = resp.text().await.unwrap_or_else(|_| "Unknown error".to_string());
        Err(format!("Validation failed (Status {}): {}", status, error_text))
    }
}

fn push_text(segments: &mut Vec<String>, text: &str) {

    if !text.trim().is_empty() {
        segments.push(text.to_owned());
    }
}
