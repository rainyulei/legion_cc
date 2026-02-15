//! Format transformation between Anthropic and OpenAI API formats

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

/// Convert Anthropic Messages API format to OpenAI Chat Completions format
pub fn anthropic_to_openai(body: &[u8], model_override: Option<&str>) -> Result<Vec<u8>> {
    let anthropic: Value = serde_json::from_slice(body)
        .map_err(|e| anyhow!("Failed to parse Anthropic request: {}", e))?;

    let mut messages = Vec::new();

    // Convert system message if present
    if let Some(system) = anthropic.get("system") {
        if let Some(system_str) = system.as_str() {
            messages.push(json!({
                "role": "system",
                "content": system_str
            }));
        } else if let Some(system_arr) = system.as_array() {
            // Handle array of content blocks
            let content: Vec<&str> = system_arr
                .iter()
                .filter_map(|block| block.get("text").and_then(|t| t.as_str()))
                .collect();
            if !content.is_empty() {
                messages.push(json!({
                    "role": "system",
                    "content": content.join("\n")
                }));
            }
        }
    }

    // Convert messages array
    if let Some(anthropic_messages) = anthropic.get("messages").and_then(|m| m.as_array()) {
        for msg in anthropic_messages {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");

            // Handle content - can be string or array of content blocks
            let content = if let Some(content_str) = msg.get("content").and_then(|c| c.as_str()) {
                content_str.to_string()
            } else if let Some(content_arr) = msg.get("content").and_then(|c| c.as_array()) {
                content_arr
                    .iter()
                    .filter_map(|block| {
                        if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                            block.get("text").and_then(|t| t.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("")
            } else {
                continue;
            };

            messages.push(json!({
                "role": role,
                "content": content
            }));
        }
    }

    // Determine model
    let model = model_override.unwrap_or_else(|| {
        anthropic
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or("gpt-4")
    });

    // Get max_tokens (OpenAI uses max_tokens too, but may need adjustment)
    let max_tokens = anthropic
        .get("max_tokens")
        .and_then(|m| m.as_u64())
        .unwrap_or(4096);

    // Check if streaming
    let stream = anthropic
        .get("stream")
        .and_then(|s| s.as_bool())
        .unwrap_or(false);

    let openai_request = json!({
        "model": model,
        "messages": messages,
        "max_tokens": max_tokens,
        "stream": stream
    });

    serde_json::to_vec(&openai_request)
        .map_err(|e| anyhow!("Failed to serialize OpenAI request: {}", e))
}

/// Convert OpenAI Chat Completions response to Anthropic Messages API format
pub fn openai_to_anthropic(body: &[u8]) -> Result<Vec<u8>> {
    let openai: Value = serde_json::from_slice(body)
        .map_err(|e| anyhow!("Failed to parse OpenAI response: {}", e))?;

    // Extract content from choices[0].message.content
    let content = openai
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|msg| msg.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("");

    // Extract usage information
    let usage = openai.get("usage").cloned().unwrap_or_else(|| {
        json!({
            "prompt_tokens": 0,
            "completion_tokens": 0,
            "total_tokens": 0
        })
    });

    // Convert usage format
    let input_tokens = usage
        .get("prompt_tokens")
        .and_then(|t| t.as_u64())
        .unwrap_or(0);
    let output_tokens = usage
        .get("completion_tokens")
        .and_then(|t| t.as_u64())
        .unwrap_or(0);

    // Generate a message ID
    let id = openai
        .get("id")
        .and_then(|i| i.as_str())
        .map(|s| format!("msg_{}", s))
        .unwrap_or_else(|| format!("msg_{}", uuid_v4_simple()));

    // Get model from response
    let model = openai
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("claude-3-5-sonnet-20241022");

    // Determine stop reason
    let stop_reason = openai
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|choice| choice.get("finish_reason"))
        .and_then(|r| r.as_str())
        .map(|reason| match reason {
            "stop" => "end_turn",
            "length" => "max_tokens",
            "content_filter" => "end_turn",
            _ => "end_turn",
        })
        .unwrap_or("end_turn");

    let anthropic_response = json!({
        "id": id,
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": [
            {
                "type": "text",
                "text": content
            }
        ],
        "stop_reason": stop_reason,
        "stop_sequence": null,
        "usage": {
            "input_tokens": input_tokens,
            "output_tokens": output_tokens
        }
    });

    serde_json::to_vec(&anthropic_response)
        .map_err(|e| anyhow!("Failed to serialize Anthropic response: {}", e))
}

/// Wrap a non-streaming Anthropic Messages API JSON response as SSE events.
///
/// Used when we forced stream=false on the upstream (openai_chat) but Claude Code
/// expects a streaming (SSE) response in Anthropic format.
pub fn wrap_anthropic_json_as_sse(anthropic_json: &[u8]) -> Result<Vec<u8>> {
    let msg: Value = serde_json::from_slice(anthropic_json)
        .map_err(|e| anyhow!("Failed to parse Anthropic response for SSE wrapping: {}", e))?;

    let id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("msg_unknown");
    let model = msg.get("model").and_then(|v| v.as_str()).unwrap_or("unknown");
    let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("assistant");
    let stop_reason = msg.get("stop_reason").and_then(|v| v.as_str()).unwrap_or("end_turn");
    let usage = msg.get("usage").cloned().unwrap_or(json!({"input_tokens": 0, "output_tokens": 0}));
    let input_tokens = usage.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    let output_tokens = usage.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);

    let mut sse = String::new();

    // message_start
    let message_start = json!({
        "type": "message_start",
        "message": {
            "id": id,
            "type": "message",
            "role": role,
            "content": [],
            "model": model,
            "stop_reason": null,
            "stop_sequence": null,
            "usage": {"input_tokens": input_tokens, "output_tokens": 0}
        }
    });
    sse.push_str(&format!("event: message_start\ndata: {}\n\n", serde_json::to_string(&message_start).unwrap()));

    // Process content blocks
    if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
        for (i, block) in content.iter().enumerate() {
            let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("text");
            if block_type == "text" {
                let text = block.get("text").and_then(|t| t.as_str()).unwrap_or("");

                // content_block_start
                let block_start = json!({
                    "type": "content_block_start",
                    "index": i,
                    "content_block": {"type": "text", "text": ""}
                });
                sse.push_str(&format!("event: content_block_start\ndata: {}\n\n", serde_json::to_string(&block_start).unwrap()));

                // content_block_delta (send all text at once)
                let delta = json!({
                    "type": "content_block_delta",
                    "index": i,
                    "delta": {"type": "text_delta", "text": text}
                });
                sse.push_str(&format!("event: content_block_delta\ndata: {}\n\n", serde_json::to_string(&delta).unwrap()));

                // content_block_stop
                let block_stop = json!({
                    "type": "content_block_stop",
                    "index": i
                });
                sse.push_str(&format!("event: content_block_stop\ndata: {}\n\n", serde_json::to_string(&block_stop).unwrap()));
            }
        }
    }

    // message_delta
    let msg_delta = json!({
        "type": "message_delta",
        "delta": {"stop_reason": stop_reason, "stop_sequence": null},
        "usage": {"output_tokens": output_tokens}
    });
    sse.push_str(&format!("event: message_delta\ndata: {}\n\n", serde_json::to_string(&msg_delta).unwrap()));

    // message_stop
    sse.push_str("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n");

    Ok(sse.into_bytes())
}

/// Generate a simple UUID v4-like string (not cryptographically secure)
fn uuid_v4_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:032x}", timestamp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anthropic_to_openai_basic() {
        let anthropic = json!({
            "model": "claude-3-5-sonnet-20241022",
            "max_tokens": 1024,
            "messages": [
                {"role": "user", "content": "Hello!"}
            ]
        });

        let result = anthropic_to_openai(anthropic.to_string().as_bytes(), None).unwrap();
        let openai: Value = serde_json::from_slice(&result).unwrap();

        assert_eq!(openai["model"], "claude-3-5-sonnet-20241022");
        assert_eq!(openai["max_tokens"], 1024);
        assert_eq!(openai["messages"][0]["role"], "user");
        assert_eq!(openai["messages"][0]["content"], "Hello!");
    }

    #[test]
    fn test_anthropic_to_openai_with_system() {
        let anthropic = json!({
            "model": "claude-3-5-sonnet-20241022",
            "max_tokens": 1024,
            "system": "You are a helpful assistant.",
            "messages": [
                {"role": "user", "content": "Hello!"}
            ]
        });

        let result = anthropic_to_openai(anthropic.to_string().as_bytes(), None).unwrap();
        let openai: Value = serde_json::from_slice(&result).unwrap();

        assert_eq!(openai["messages"][0]["role"], "system");
        assert_eq!(openai["messages"][0]["content"], "You are a helpful assistant.");
        assert_eq!(openai["messages"][1]["role"], "user");
    }

    #[test]
    fn test_anthropic_to_openai_with_model_override() {
        let anthropic = json!({
            "model": "claude-3-5-sonnet-20241022",
            "max_tokens": 1024,
            "messages": [
                {"role": "user", "content": "Hello!"}
            ]
        });

        let result = anthropic_to_openai(
            anthropic.to_string().as_bytes(),
            Some("gpt-4-turbo"),
        )
        .unwrap();
        let openai: Value = serde_json::from_slice(&result).unwrap();

        assert_eq!(openai["model"], "gpt-4-turbo");
    }

    #[test]
    fn test_openai_to_anthropic_basic() {
        let openai = json!({
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "model": "gpt-4",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "Hello! How can I help you?"
                    },
                    "finish_reason": "stop"
                }
            ],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 20,
                "total_tokens": 30
            }
        });

        let result = openai_to_anthropic(openai.to_string().as_bytes()).unwrap();
        let anthropic: Value = serde_json::from_slice(&result).unwrap();

        assert_eq!(anthropic["type"], "message");
        assert_eq!(anthropic["role"], "assistant");
        assert_eq!(anthropic["content"][0]["type"], "text");
        assert_eq!(anthropic["content"][0]["text"], "Hello! How can I help you?");
        assert_eq!(anthropic["stop_reason"], "end_turn");
        assert_eq!(anthropic["usage"]["input_tokens"], 10);
        assert_eq!(anthropic["usage"]["output_tokens"], 20);
    }
}
