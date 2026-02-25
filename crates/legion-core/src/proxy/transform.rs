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
            if let Some(content_str) = msg.get("content").and_then(|c| c.as_str()) {
                messages.push(json!({
                    "role": role,
                    "content": content_str
                }));
            } else if let Some(content_arr) = msg.get("content").and_then(|c| c.as_array()) {
                // Separate text, tool_use, and tool_result blocks
                let mut text_parts: Vec<String> = Vec::new();
                let mut tool_calls: Vec<Value> = Vec::new();
                let mut tool_results: Vec<(String, String)> = Vec::new(); // (tool_call_id, content)

                for block in content_arr {
                    match block.get("type").and_then(|t| t.as_str()) {
                        Some("text") => {
                            if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                text_parts.push(text.to_string());
                            }
                        }
                        Some("tool_use") => {
                            // Anthropic tool_use → OpenAI tool_call
                            let id = block.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let input = block.get("input").cloned().unwrap_or(json!({}));
                            tool_calls.push(json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": serde_json::to_string(&input).unwrap_or_default()
                                }
                            }));
                        }
                        Some("tool_result") => {
                            let tool_use_id = block.get("tool_use_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            // Content can be string or array of content blocks
                            let content = if let Some(s) = block.get("content").and_then(|c| c.as_str()) {
                                s.to_string()
                            } else if let Some(arr) = block.get("content").and_then(|c| c.as_array()) {
                                arr.iter()
                                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                                    .collect::<Vec<_>>()
                                    .join("")
                            } else {
                                String::new()
                            };
                            tool_results.push((tool_use_id, content));
                        }
                        _ => {
                            // Skip thinking and other block types
                        }
                    }
                }

                // Emit assistant message with tool_calls if present
                if role == "assistant" && !tool_calls.is_empty() {
                    let mut assistant_msg = json!({
                        "role": "assistant",
                        "tool_calls": tool_calls
                    });
                    if !text_parts.is_empty() {
                        assistant_msg["content"] = json!(text_parts.join(""));
                    }
                    messages.push(assistant_msg);
                }

                // Emit tool result messages BEFORE any user text,
                // so they immediately follow the assistant's tool_calls
                // (required by minimax and other OpenAI-compatible APIs)
                for (tool_call_id, content) in &tool_results {
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": tool_call_id,
                        "content": content
                    }));
                }

                // Emit text for non-assistant roles (user text after tool results)
                if role != "assistant" && !text_parts.is_empty() {
                    messages.push(json!({
                        "role": role,
                        "content": text_parts.join("")
                    }));
                }
            } else {
                continue;
            }
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

    let mut openai_request = json!({
        "model": model,
        "messages": messages,
        "max_tokens": max_tokens,
        "stream": stream
    });

    // Convert Anthropic tools to OpenAI tools (function calling)
    if let Some(tools) = anthropic.get("tools").and_then(|t| t.as_array()) {
        let openai_tools: Vec<Value> = tools
            .iter()
            .map(|tool| {
                let name = tool.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let description = tool.get("description").and_then(|d| d.as_str()).unwrap_or("");
                let parameters = tool.get("input_schema").cloned().unwrap_or(json!({"type": "object"}));
                json!({
                    "type": "function",
                    "function": {
                        "name": name,
                        "description": description,
                        "parameters": parameters
                    }
                })
            })
            .collect();
        if !openai_tools.is_empty() {
            openai_request["tools"] = json!(openai_tools);
        }
    }

    serde_json::to_vec(&openai_request)
        .map_err(|e| anyhow!("Failed to serialize OpenAI request: {}", e))
}

/// Convert OpenAI Chat Completions response to Anthropic Messages API format
pub fn openai_to_anthropic(body: &[u8]) -> Result<Vec<u8>> {
    let openai: Value = serde_json::from_slice(body)
        .map_err(|e| anyhow!("Failed to parse OpenAI response: {}", e))?;

    let choices = openai
        .get("choices")
        .and_then(|c| c.as_array());

    // Build content blocks by merging ALL choices
    // (Copilot may split text and tool_calls into separate choice objects)
    let mut content_blocks: Vec<Value> = Vec::new();
    let mut final_finish_reason = "stop";

    if let Some(choices_arr) = choices {
        for choice in choices_arr {
            let message = choice.get("message");

            // Extract text content
            if let Some(text) = message.and_then(|m| m.get("content")).and_then(|c| c.as_str()) {
                if !text.is_empty() {
                    content_blocks.push(json!({
                        "type": "text",
                        "text": text
                    }));
                }
            }

            // Extract tool_calls → Anthropic tool_use blocks
            if let Some(tool_calls) = message.and_then(|m| m.get("tool_calls")).and_then(|t| t.as_array()) {
                for tc in tool_calls {
                    let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let empty_obj = json!({});
                    let func = tc.get("function").unwrap_or(&empty_obj);
                    let name = func.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let args_str = func.get("arguments").and_then(|a| a.as_str()).unwrap_or("{}");
                    let input: Value = serde_json::from_str(args_str).unwrap_or(json!({}));

                    content_blocks.push(json!({
                        "type": "tool_use",
                        "id": id,
                        "name": name,
                        "input": input
                    }));
                }
            }

            // Use the most specific finish_reason (tool_calls > stop > length)
            if let Some(fr) = choice.get("finish_reason").and_then(|r| r.as_str()) {
                match fr {
                    "tool_calls" => final_finish_reason = "tool_calls",
                    "stop" if final_finish_reason != "tool_calls" => final_finish_reason = "stop",
                    "length" if final_finish_reason == "stop" => final_finish_reason = "length",
                    _ => {}
                }
            }
        }
    }

    // If no content blocks at all, add an empty text block
    if content_blocks.is_empty() {
        content_blocks.push(json!({
            "type": "text",
            "text": ""
        }));
    }

    // Extract usage information
    let usage = openai.get("usage").cloned().unwrap_or_else(|| {
        json!({
            "prompt_tokens": 0,
            "completion_tokens": 0,
            "total_tokens": 0
        })
    });

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

    // Map finish_reason to Anthropic stop_reason
    let finish_reason = final_finish_reason;

    let stop_reason = match finish_reason {
        "stop" => "end_turn",
        "length" => "max_tokens",
        "tool_calls" => "tool_use",
        "content_filter" => "end_turn",
        _ => "end_turn",
    };

    let anthropic_response = json!({
        "id": id,
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": content_blocks,
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
            match block_type {
                "text" => {
                    let text = block.get("text").and_then(|t| t.as_str()).unwrap_or("");

                    let block_start = json!({
                        "type": "content_block_start",
                        "index": i,
                        "content_block": {"type": "text", "text": ""}
                    });
                    sse.push_str(&format!("event: content_block_start\ndata: {}\n\n", serde_json::to_string(&block_start).unwrap()));

                    let delta = json!({
                        "type": "content_block_delta",
                        "index": i,
                        "delta": {"type": "text_delta", "text": text}
                    });
                    sse.push_str(&format!("event: content_block_delta\ndata: {}\n\n", serde_json::to_string(&delta).unwrap()));

                    let block_stop = json!({
                        "type": "content_block_stop",
                        "index": i
                    });
                    sse.push_str(&format!("event: content_block_stop\ndata: {}\n\n", serde_json::to_string(&block_stop).unwrap()));
                }
                "tool_use" => {
                    let tool_id = block.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let tool_name = block.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let input = block.get("input").cloned().unwrap_or(json!({}));

                    let block_start = json!({
                        "type": "content_block_start",
                        "index": i,
                        "content_block": {
                            "type": "tool_use",
                            "id": tool_id,
                            "name": tool_name,
                            "input": {}
                        }
                    });
                    sse.push_str(&format!("event: content_block_start\ndata: {}\n\n", serde_json::to_string(&block_start).unwrap()));

                    // Send input as a single JSON delta
                    let input_str = serde_json::to_string(&input).unwrap_or_default();
                    let delta = json!({
                        "type": "content_block_delta",
                        "index": i,
                        "delta": {"type": "input_json_delta", "partial_json": input_str}
                    });
                    sse.push_str(&format!("event: content_block_delta\ndata: {}\n\n", serde_json::to_string(&delta).unwrap()));

                    let block_stop = json!({
                        "type": "content_block_stop",
                        "index": i
                    });
                    sse.push_str(&format!("event: content_block_stop\ndata: {}\n\n", serde_json::to_string(&block_stop).unwrap()));
                }
                _ => {}
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

/// Convert Anthropic Messages API format to OpenAI Responses API format (`/v1/responses`)
///
/// Key differences from Chat Completions:
/// - `system` → `instructions`
/// - `messages` → `input` (with different item types for tool calls/results)
/// - `max_tokens` → `max_output_tokens`
/// - Tool use → `function_call` items, Tool result → `function_call_output` items
pub fn anthropic_to_openai_responses(body: &[u8], model_override: Option<&str>) -> Result<Vec<u8>> {
    let anthropic: Value = serde_json::from_slice(body)
        .map_err(|e| anyhow!("Failed to parse Anthropic request: {}", e))?;

    let mut input: Vec<Value> = Vec::new();

    // Convert system → instructions
    let instructions = if let Some(system) = anthropic.get("system") {
        if let Some(system_str) = system.as_str() {
            Some(system_str.to_string())
        } else if let Some(system_arr) = system.as_array() {
            let content: Vec<&str> = system_arr
                .iter()
                .filter_map(|block| block.get("text").and_then(|t| t.as_str()))
                .collect();
            if content.is_empty() { None } else { Some(content.join("\n")) }
        } else {
            None
        }
    } else {
        None
    };

    // Convert messages → input
    if let Some(anthropic_messages) = anthropic.get("messages").and_then(|m| m.as_array()) {
        for msg in anthropic_messages {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");

            if let Some(content_str) = msg.get("content").and_then(|c| c.as_str()) {
                input.push(json!({"role": role, "content": content_str}));
            } else if let Some(content_arr) = msg.get("content").and_then(|c| c.as_array()) {
                let mut text_parts: Vec<String> = Vec::new();

                for block in content_arr {
                    match block.get("type").and_then(|t| t.as_str()) {
                        Some("text") => {
                            if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                text_parts.push(text.to_string());
                            }
                        }
                        Some("tool_use") => {
                            // Emit accumulated text first
                            if !text_parts.is_empty() {
                                input.push(json!({"role": role, "content": text_parts.join("")}));
                                text_parts.clear();
                            }
                            // Anthropic tool_use → Responses API function_call item
                            let id = block.get("id").and_then(|v| v.as_str()).unwrap_or("");
                            let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("");
                            let args = block.get("input").cloned().unwrap_or(json!({}));
                            input.push(json!({
                                "type": "function_call",
                                "call_id": id,
                                "name": name,
                                "arguments": serde_json::to_string(&args).unwrap_or_default()
                            }));
                        }
                        Some("tool_result") => {
                            // Emit accumulated text first
                            if !text_parts.is_empty() {
                                input.push(json!({"role": role, "content": text_parts.join("")}));
                                text_parts.clear();
                            }
                            // Anthropic tool_result → Responses API function_call_output item
                            let tool_use_id = block.get("tool_use_id").and_then(|v| v.as_str()).unwrap_or("");
                            let content = if let Some(s) = block.get("content").and_then(|c| c.as_str()) {
                                s.to_string()
                            } else if let Some(arr) = block.get("content").and_then(|c| c.as_array()) {
                                arr.iter()
                                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                                    .collect::<Vec<_>>()
                                    .join("")
                            } else {
                                String::new()
                            };
                            input.push(json!({
                                "type": "function_call_output",
                                "call_id": tool_use_id,
                                "output": content
                            }));
                        }
                        _ => {
                            // Skip thinking and other block types
                        }
                    }
                }

                // Emit remaining text
                if !text_parts.is_empty() {
                    input.push(json!({"role": role, "content": text_parts.join("")}));
                }
            }
        }
    }

    // Model
    let model = model_override.unwrap_or_else(|| {
        anthropic
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or("gpt-4")
    });

    // max_tokens → max_output_tokens
    let max_output_tokens = anthropic
        .get("max_tokens")
        .and_then(|m| m.as_u64())
        .unwrap_or(4096);

    let stream = anthropic
        .get("stream")
        .and_then(|s| s.as_bool())
        .unwrap_or(false);

    let mut request = json!({
        "model": model,
        "input": input,
        "max_output_tokens": max_output_tokens,
        "stream": stream
    });

    // instructions (system prompt)
    if let Some(ref inst) = instructions {
        request["instructions"] = json!(inst);
    }

    // Convert tools (same structure as chat completions: {type: "function", function: {...}})
    // OpenAI strict mode requires ALL properties to be in `required` array,
    // so we patch schemas to ensure completeness.
    if let Some(tools) = anthropic.get("tools").and_then(|t| t.as_array()) {
        let resp_tools: Vec<Value> = tools
            .iter()
            .map(|tool| {
                let name = tool.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let description = tool.get("description").and_then(|d| d.as_str()).unwrap_or("");
                let mut parameters = tool.get("input_schema").cloned().unwrap_or(json!({"type": "object"}));
                // Patch: ensure `required` includes ALL property keys for strict mode
                ensure_all_properties_required(&mut parameters);
                json!({
                    "type": "function",
                    "name": name,
                    "description": description,
                    "parameters": parameters,
                    "strict": true
                })
            })
            .collect();
        if !resp_tools.is_empty() {
            request["tools"] = json!(resp_tools);
        }
    }

    serde_json::to_vec(&request)
        .map_err(|e| anyhow!("Failed to serialize Responses API request: {}", e))
}

/// Convert OpenAI Responses API response to Anthropic Messages API format
///
/// Response format:
/// ```json
/// { "output": [ {"type": "message", "content": [{"type": "output_text", "text": "..."}]},
///               {"type": "function_call", "call_id": "...", "name": "...", "arguments": "..."} ],
///   "usage": {"input_tokens": N, "output_tokens": M} }
/// ```
pub fn openai_responses_to_anthropic(body: &[u8]) -> Result<Vec<u8>> {
    let resp: Value = serde_json::from_slice(body)
        .map_err(|e| anyhow!("Failed to parse Responses API response: {}", e))?;

    let mut content_blocks: Vec<Value> = Vec::new();
    let mut has_function_call = false;

    if let Some(output) = resp.get("output").and_then(|o| o.as_array()) {
        for item in output {
            match item.get("type").and_then(|t| t.as_str()) {
                Some("message") => {
                    // Message item: extract content blocks
                    if let Some(content) = item.get("content").and_then(|c| c.as_array()) {
                        for block in content {
                            match block.get("type").and_then(|t| t.as_str()) {
                                Some("output_text") => {
                                    let text = block.get("text").and_then(|t| t.as_str()).unwrap_or("");
                                    if !text.is_empty() {
                                        content_blocks.push(json!({"type": "text", "text": text}));
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    // Also handle content as string
                    if let Some(text) = item.get("content").and_then(|c| c.as_str()) {
                        if !text.is_empty() {
                            content_blocks.push(json!({"type": "text", "text": text}));
                        }
                    }
                }
                Some("function_call") => {
                    has_function_call = true;
                    let call_id = item.get("call_id")
                        .or_else(|| item.get("id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let args_str = item.get("arguments").and_then(|a| a.as_str()).unwrap_or("{}");
                    let input: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
                    content_blocks.push(json!({
                        "type": "tool_use",
                        "id": call_id,
                        "name": name,
                        "input": input
                    }));
                }
                _ => {}
            }
        }
    }

    if content_blocks.is_empty() {
        content_blocks.push(json!({"type": "text", "text": ""}));
    }

    // Usage — Responses API uses input_tokens/output_tokens directly
    let usage = resp.get("usage").cloned().unwrap_or(json!({}));
    let input_tokens = usage.get("input_tokens").and_then(|t| t.as_u64()).unwrap_or(0);
    let output_tokens = usage.get("output_tokens").and_then(|t| t.as_u64()).unwrap_or(0);

    let id = resp.get("id").and_then(|i| i.as_str())
        .map(|s| format!("msg_{}", s))
        .unwrap_or_else(|| format!("msg_{}", uuid_v4_simple()));

    let model = resp.get("model").and_then(|m| m.as_str()).unwrap_or("unknown");

    // Map status to stop_reason
    let stop_reason = if has_function_call {
        "tool_use"
    } else {
        match resp.get("status").and_then(|s| s.as_str()) {
            Some("completed") => "end_turn",
            Some("incomplete") => "max_tokens",
            _ => "end_turn",
        }
    };

    let anthropic_response = json!({
        "id": id,
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": content_blocks,
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

/// Whitelist of JSON Schema keywords supported by OpenAI strict mode.
/// Everything not in this list gets stripped to avoid schema validation errors.
const STRICT_MODE_ALLOWED_KEYS: &[&str] = &[
    "type", "properties", "required", "items", "additionalProperties",
    "enum", "const", "anyOf", "$ref", "$defs", "description", "title", "default",
];

/// Recursively sanitize a JSON Schema for OpenAI strict mode:
/// 1. Strip all unsupported keywords (whitelist approach)
/// 2. Ensure `required` includes ALL property keys
/// 3. Set `additionalProperties: false` on all objects
fn ensure_all_properties_required(schema: &mut Value) {
    let obj = match schema.as_object_mut() {
        Some(o) => o,
        None => return,
    };

    // Strip unsupported keywords
    obj.retain(|key, _| STRICT_MODE_ALLOWED_KEYS.contains(&key.as_str()));

    // Ensure all property keys are in `required`
    if let Some(props_keys) = obj.get("properties").and_then(|p| p.as_object()).map(|o| {
        o.keys().cloned().collect::<Vec<String>>()
    }) {
        let required = props_keys.iter().map(|k| json!(k)).collect::<Vec<Value>>();
        obj.insert("required".to_string(), json!(required));
    }

    // Recurse into nested schemas within `properties`
    if let Some(props_obj) = obj.get_mut("properties").and_then(|p| p.as_object_mut()) {
        for (_key, prop_schema) in props_obj.iter_mut() {
            ensure_all_properties_required(prop_schema);
        }
    }

    // Recurse into `items` (array element schema)
    if let Some(items) = obj.get_mut("items") {
        ensure_all_properties_required(items);
    }

    // Recurse into `anyOf` variants
    if let Some(any_of) = obj.get_mut("anyOf").and_then(|v| v.as_array_mut()) {
        for variant in any_of.iter_mut() {
            ensure_all_properties_required(variant);
        }
    }

    // Recurse into `$defs`
    if let Some(defs) = obj.get_mut("$defs").and_then(|v| v.as_object_mut()) {
        for (_name, def_schema) in defs.iter_mut() {
            ensure_all_properties_required(def_schema);
        }
    }

    // All objects must have properties, required, and additionalProperties: false
    if obj.get("type").and_then(|t| t.as_str()) == Some("object") {
        obj.insert("additionalProperties".to_string(), json!(false));
        // Objects without `properties` need empty ones (strict mode requires it)
        if obj.get("properties").is_none() {
            obj.insert("properties".to_string(), json!({}));
            obj.insert("required".to_string(), json!([]));
        }
    }
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

    #[test]
    fn test_anthropic_to_openai_with_tools() {
        let anthropic = json!({
            "model": "claude-3-5-sonnet-20241022",
            "max_tokens": 1024,
            "tools": [
                {
                    "name": "Write",
                    "description": "Writes a file",
                    "input_schema": {
                        "type": "object",
                        "properties": {
                            "file_path": {"type": "string"},
                            "content": {"type": "string"}
                        },
                        "required": ["file_path", "content"]
                    }
                }
            ],
            "messages": [
                {"role": "user", "content": "Create a file"}
            ]
        });

        let result = anthropic_to_openai(anthropic.to_string().as_bytes(), None).unwrap();
        let openai: Value = serde_json::from_slice(&result).unwrap();

        assert!(openai.get("tools").is_some());
        let tools = openai["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["function"]["name"], "Write");
        assert_eq!(tools[0]["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn test_anthropic_to_openai_tool_use_messages() {
        let anthropic = json!({
            "model": "claude-3-5-sonnet-20241022",
            "max_tokens": 1024,
            "messages": [
                {"role": "user", "content": "Create a file"},
                {
                    "role": "assistant",
                    "content": [
                        {"type": "text", "text": "I'll create the file."},
                        {
                            "type": "tool_use",
                            "id": "toolu_123",
                            "name": "Write",
                            "input": {"file_path": "/tmp/test.py", "content": "print('hello')"}
                        }
                    ]
                },
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "toolu_123",
                            "content": "File written successfully"
                        }
                    ]
                }
            ]
        });

        let result = anthropic_to_openai(anthropic.to_string().as_bytes(), None).unwrap();
        let openai: Value = serde_json::from_slice(&result).unwrap();

        let messages = openai["messages"].as_array().unwrap();
        // user, assistant (with tool_calls), tool result
        assert_eq!(messages.len(), 3);

        // Assistant message should have tool_calls
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"], "I'll create the file.");
        let tool_calls = messages[1]["tool_calls"].as_array().unwrap();
        assert_eq!(tool_calls[0]["function"]["name"], "Write");
        assert_eq!(tool_calls[0]["id"], "toolu_123");

        // Tool result message
        assert_eq!(messages[2]["role"], "tool");
        assert_eq!(messages[2]["tool_call_id"], "toolu_123");
        assert_eq!(messages[2]["content"], "File written successfully");
    }

    #[test]
    fn test_openai_to_anthropic_with_tool_calls() {
        let openai = json!({
            "id": "chatcmpl-456",
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_abc",
                        "type": "function",
                        "function": {
                            "name": "Write",
                            "arguments": "{\"file_path\":\"/tmp/test.py\",\"content\":\"print('hello')\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 20, "total_tokens": 30}
        });

        let result = openai_to_anthropic(openai.to_string().as_bytes()).unwrap();
        let anthropic: Value = serde_json::from_slice(&result).unwrap();

        assert_eq!(anthropic["stop_reason"], "tool_use");
        let content = anthropic["content"].as_array().unwrap();
        assert_eq!(content.len(), 1); // Only tool_use, no text (content was null)
        assert_eq!(content[0]["type"], "tool_use");
        assert_eq!(content[0]["name"], "Write");
        assert_eq!(content[0]["id"], "call_abc");
        assert_eq!(content[0]["input"]["file_path"], "/tmp/test.py");
    }

    #[test]
    fn test_anthropic_to_openai_responses_basic() {
        let anthropic = json!({
            "model": "gpt-5.2-codex",
            "max_tokens": 2048,
            "system": "You are a helpful assistant.",
            "messages": [
                {"role": "user", "content": "Hello!"}
            ]
        });

        let result = anthropic_to_openai_responses(anthropic.to_string().as_bytes(), None).unwrap();
        let resp: Value = serde_json::from_slice(&result).unwrap();

        assert_eq!(resp["model"], "gpt-5.2-codex");
        assert_eq!(resp["max_output_tokens"], 2048);
        assert_eq!(resp["instructions"], "You are a helpful assistant.");
        assert_eq!(resp["input"][0]["role"], "user");
        assert_eq!(resp["input"][0]["content"], "Hello!");
        // Should not have "messages" key
        assert!(resp.get("messages").is_none());
    }

    #[test]
    fn test_anthropic_to_openai_responses_with_tools() {
        let anthropic = json!({
            "model": "gpt-5.2-codex",
            "max_tokens": 1024,
            "messages": [
                {"role": "user", "content": "Create a file"},
                {
                    "role": "assistant",
                    "content": [
                        {"type": "text", "text": "I'll create it."},
                        {
                            "type": "tool_use",
                            "id": "toolu_123",
                            "name": "Write",
                            "input": {"file_path": "/tmp/test.py", "content": "print('hello')"}
                        }
                    ]
                },
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "toolu_123",
                            "content": "File written successfully"
                        }
                    ]
                }
            ]
        });

        let result = anthropic_to_openai_responses(anthropic.to_string().as_bytes(), None).unwrap();
        let resp: Value = serde_json::from_slice(&result).unwrap();

        let input = resp["input"].as_array().unwrap();
        // user, assistant text, function_call, function_call_output
        assert_eq!(input.len(), 4);
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[1]["role"], "assistant");
        assert_eq!(input[1]["content"], "I'll create it.");
        assert_eq!(input[2]["type"], "function_call");
        assert_eq!(input[2]["call_id"], "toolu_123");
        assert_eq!(input[2]["name"], "Write");
        assert_eq!(input[3]["type"], "function_call_output");
        assert_eq!(input[3]["call_id"], "toolu_123");
        assert_eq!(input[3]["output"], "File written successfully");
    }

    #[test]
    fn test_openai_responses_to_anthropic_basic() {
        let resp = json!({
            "id": "resp_abc123",
            "object": "response",
            "status": "completed",
            "model": "gpt-5.2-codex",
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        {"type": "output_text", "text": "Hello! How can I help?"}
                    ]
                }
            ],
            "usage": {
                "input_tokens": 15,
                "output_tokens": 25
            }
        });

        let result = openai_responses_to_anthropic(resp.to_string().as_bytes()).unwrap();
        let anthropic: Value = serde_json::from_slice(&result).unwrap();

        assert_eq!(anthropic["type"], "message");
        assert_eq!(anthropic["role"], "assistant");
        assert_eq!(anthropic["model"], "gpt-5.2-codex");
        assert_eq!(anthropic["content"][0]["type"], "text");
        assert_eq!(anthropic["content"][0]["text"], "Hello! How can I help?");
        assert_eq!(anthropic["stop_reason"], "end_turn");
        assert_eq!(anthropic["usage"]["input_tokens"], 15);
        assert_eq!(anthropic["usage"]["output_tokens"], 25);
    }

    #[test]
    fn test_openai_responses_to_anthropic_with_function_call() {
        let resp = json!({
            "id": "resp_456",
            "status": "completed",
            "model": "gpt-5.2-codex",
            "output": [
                {
                    "type": "function_call",
                    "call_id": "call_abc",
                    "name": "Write",
                    "arguments": "{\"file_path\":\"/tmp/test.py\",\"content\":\"print('hello')\"}"
                }
            ],
            "usage": {"input_tokens": 10, "output_tokens": 20}
        });

        let result = openai_responses_to_anthropic(resp.to_string().as_bytes()).unwrap();
        let anthropic: Value = serde_json::from_slice(&result).unwrap();

        assert_eq!(anthropic["stop_reason"], "tool_use");
        let content = anthropic["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "tool_use");
        assert_eq!(content[0]["name"], "Write");
        assert_eq!(content[0]["id"], "call_abc");
        assert_eq!(content[0]["input"]["file_path"], "/tmp/test.py");
    }

    #[test]
    fn test_responses_api_strict_mode_all_properties_required() {
        // Simulate the Task tool schema where `isolation` is optional (not in required)
        let anthropic = json!({
            "model": "gpt-5.2-codex",
            "max_tokens": 1024,
            "tools": [
                {
                    "name": "Task",
                    "description": "Launch a subagent",
                    "input_schema": {
                        "type": "object",
                        "properties": {
                            "description": {"type": "string"},
                            "prompt": {"type": "string"},
                            "subagent_type": {"type": "string"},
                            "isolation": {
                                "type": "string",
                                "enum": ["worktree"]
                            }
                        },
                        "required": ["description", "prompt", "subagent_type"]
                    }
                }
            ],
            "messages": [
                {"role": "user", "content": "Hello"}
            ]
        });

        let result = anthropic_to_openai_responses(anthropic.to_string().as_bytes(), None).unwrap();
        let resp: Value = serde_json::from_slice(&result).unwrap();

        let tool = &resp["tools"][0];
        assert_eq!(tool["strict"], true);

        // All 4 properties must be in required (including optional `isolation`)
        let required = tool["parameters"]["required"].as_array().unwrap();
        let required_strs: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(required_strs.contains(&"description"));
        assert!(required_strs.contains(&"prompt"));
        assert!(required_strs.contains(&"subagent_type"));
        assert!(required_strs.contains(&"isolation"), "isolation must be in required for strict mode");

        // additionalProperties should be false
        assert_eq!(tool["parameters"]["additionalProperties"], false);
    }

    #[test]
    fn test_responses_api_strict_mode_strips_unsupported_keywords() {
        // Simulate TaskCreate schema with propertyNames and other unsupported keywords
        let anthropic = json!({
            "model": "gpt-5.2-codex",
            "max_tokens": 1024,
            "tools": [
                {
                    "name": "TaskCreate",
                    "description": "Create a task",
                    "input_schema": {
                        "$schema": "https://json-schema.org/draft/2020-12/schema",
                        "type": "object",
                        "properties": {
                            "subject": {"type": "string", "minLength": 1, "maxLength": 200},
                            "description": {"type": "string"},
                            "metadata": {
                                "type": "object",
                                "additionalProperties": {},
                                "propertyNames": {"type": "string"}
                            }
                        },
                        "required": ["subject", "description"]
                    }
                }
            ],
            "messages": [
                {"role": "user", "content": "Hello"}
            ]
        });

        let result = anthropic_to_openai_responses(anthropic.to_string().as_bytes(), None).unwrap();
        let resp: Value = serde_json::from_slice(&result).unwrap();

        let params = &resp["tools"][0]["parameters"];

        // $schema should be stripped
        assert!(params.get("$schema").is_none(), "$schema must be stripped");

        // subject should lose minLength/maxLength
        let subject = &params["properties"]["subject"];
        assert!(subject.get("minLength").is_none(), "minLength must be stripped");
        assert!(subject.get("maxLength").is_none(), "maxLength must be stripped");
        assert_eq!(subject["type"], "string"); // type preserved

        // metadata should lose propertyNames, additionalProperties forced to false,
        // and gain empty properties + required (strict mode requires them for objects)
        let metadata = &params["properties"]["metadata"];
        assert!(metadata.get("propertyNames").is_none(), "propertyNames must be stripped");
        assert_eq!(metadata["additionalProperties"], false);
        assert_eq!(metadata["type"], "object");
        assert_eq!(metadata["properties"], json!({}), "empty properties required for strict objects");
        assert_eq!(metadata["required"], json!([]), "empty required for strict objects without properties");

        // All 3 properties in required
        let required = params["required"].as_array().unwrap();
        let required_strs: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(required_strs.contains(&"subject"));
        assert!(required_strs.contains(&"description"));
        assert!(required_strs.contains(&"metadata"));
    }
}
