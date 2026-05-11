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

fn push_text(segments: &mut Vec<String>, text: &str) {
    if !text.trim().is_empty() {
        segments.push(text.to_owned());
    }
}
