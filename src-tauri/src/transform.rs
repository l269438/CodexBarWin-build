use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexChatReasoning {
    pub supports_thinking: bool,
    pub supports_effort: bool,
    pub thinking_param: String,
    pub effort_param: String,
    pub effort_value_mode: String,
    pub output_format: String,
}

impl CodexChatReasoning {
    pub fn deepseek() -> Self {
        Self {
            supports_thinking: true,
            supports_effort: true,
            thinking_param: "thinking".to_string(),
            effort_param: "reasoning_effort".to_string(),
            effort_value_mode: "deepseek".to_string(),
            output_format: "reasoning_content".to_string(),
        }
    }
}

pub fn responses_to_chat_completions(
    body: Value,
    upstream_model: Option<&str>,
    reasoning: Option<&CodexChatReasoning>,
) -> anyhow::Result<Value> {
    let mut result = json!({});

    let model = upstream_model
        .filter(|model| !model.trim().is_empty())
        .or_else(|| body.get("model").and_then(Value::as_str))
        .unwrap_or("deepseek-v4-flash");
    result["model"] = json!(model);

    let mut messages = Vec::new();
    if let Some(instructions) = body.get("instructions") {
        let text = instruction_text(instructions);
        if !text.trim().is_empty() {
            messages.push(json!({ "role": "system", "content": text }));
        }
    }
    if let Some(input) = body.get("input") {
        append_responses_input(input, &mut messages);
    }
    result["messages"] = json!(collapse_system_messages(messages));

    if let Some(max) = body.get("max_output_tokens") {
        result["max_tokens"] = max.clone();
    }
    for key in [
        "max_tokens",
        "max_completion_tokens",
        "temperature",
        "top_p",
        "stream",
    ] {
        if let Some(value) = body.get(key) {
            result[key] = value.clone();
        }
    }

    apply_reasoning(&mut result, &body, reasoning);

    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        let tools = tools
            .iter()
            .filter_map(responses_tool_to_chat_tool)
            .collect::<Vec<_>>();
        if !tools.is_empty() {
            result["tools"] = json!(tools);
        }
    }
    if let Some(tool_choice) = body.get("tool_choice") {
        result["tool_choice"] = responses_tool_choice_to_chat(tool_choice);
    }

    if result
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        result["stream_options"] = json!({ "include_usage": true });
    }

    Ok(result)
}

pub fn chat_completion_to_response(body: Value) -> anyhow::Result<Value> {
    let choice = body
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .ok_or_else(|| anyhow::anyhow!("missing chat choices"))?;
    let message = choice
        .get("message")
        .ok_or_else(|| anyhow::anyhow!("missing chat message"))?;

    let response_id = response_id_from_chat_id(body.get("id").and_then(Value::as_str));
    let model = body.get("model").and_then(Value::as_str).unwrap_or("");
    let created_at = body.get("created").and_then(Value::as_u64).unwrap_or(0);
    let finish_reason = choice.get("finish_reason").and_then(Value::as_str);

    let reasoning = chat_reasoning_text(message);
    let mut output = Vec::new();
    if let Some(reasoning) = reasoning.as_deref().filter(|text| !text.is_empty()) {
        output.push(json!({
            "id": format!("rs_{response_id}"),
            "type": "reasoning",
            "summary": [{ "type": "summary_text", "text": reasoning }]
        }));
    }
    if let Some(message_item) = chat_message_to_response_output_item(message, &response_id) {
        output.push(message_item);
    }
    output.extend(chat_tool_calls_to_response_items(
        message,
        reasoning.as_deref(),
    ));

    let mut response = json!({
        "id": response_id,
        "object": "response",
        "created_at": created_at,
        "status": response_status_from_finish_reason(finish_reason),
        "model": model,
        "output": output,
        "usage": chat_usage_to_responses_usage(body.get("usage")),
    });
    if finish_reason == Some("length") {
        response["incomplete_details"] = json!({ "reason": "max_output_tokens" });
    }
    Ok(response)
}

pub fn response_id_from_chat_id(id: Option<&str>) -> String {
    let id = id.unwrap_or("ccswitcher");
    if id.starts_with("resp_") {
        id.to_string()
    } else {
        format!("resp_{id}")
    }
}

pub fn response_status_from_finish_reason(reason: Option<&str>) -> &'static str {
    match reason {
        Some("length") => "incomplete",
        _ => "completed",
    }
}

pub fn chat_usage_to_responses_usage(usage: Option<&Value>) -> Value {
    let Some(usage) = usage else {
        return json!({ "input_tokens": 0, "output_tokens": 0, "total_tokens": 0 });
    };
    let input = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total = usage
        .get("total_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(input + output);
    let mut result = json!({
        "input_tokens": input,
        "output_tokens": output,
        "total_tokens": total,
    });
    if let Some(cached) = usage
        .pointer("/prompt_tokens_details/cached_tokens")
        .or_else(|| usage.pointer("/input_tokens_details/cached_tokens"))
        .and_then(Value::as_u64)
    {
        result["input_tokens_details"] = json!({ "cached_tokens": cached });
    }
    if let Some(reasoning) = usage
        .pointer("/completion_tokens_details/reasoning_tokens")
        .or_else(|| usage.pointer("/output_tokens_details/reasoning_tokens"))
        .and_then(Value::as_u64)
    {
        result["output_tokens_details"] = json!({ "reasoning_tokens": reasoning });
    }
    result
}

fn instruction_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| part.as_str())
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

fn append_responses_input(input: &Value, messages: &mut Vec<Value>) {
    match input {
        Value::String(text) => messages.push(json!({ "role": "user", "content": text })),
        Value::Array(items) => {
            let mut pending_tool_calls = Vec::new();
            for item in items {
                append_responses_item(item, messages, &mut pending_tool_calls);
            }
            flush_tool_calls(messages, &mut pending_tool_calls);
        }
        Value::Object(_) => {
            let mut pending_tool_calls = Vec::new();
            append_responses_item(input, messages, &mut pending_tool_calls);
            flush_tool_calls(messages, &mut pending_tool_calls);
        }
        _ => {}
    }
}

fn append_responses_item(
    item: &Value,
    messages: &mut Vec<Value>,
    pending_tool_calls: &mut Vec<Value>,
) {
    match item.get("type").and_then(Value::as_str) {
        Some("function_call") => pending_tool_calls.push(responses_function_call_to_chat(item)),
        Some("function_call_output") => {
            flush_tool_calls(messages, pending_tool_calls);
            messages.push(json!({
                "role": "tool",
                "tool_call_id": response_item_call_id(item),
                "content": item.get("output").and_then(Value::as_str).unwrap_or("").to_string(),
            }));
        }
        Some("message") | None => {
            flush_tool_calls(messages, pending_tool_calls);
            messages.push(responses_message_to_chat(item));
        }
        _ => {}
    }
}

fn flush_tool_calls(messages: &mut Vec<Value>, pending_tool_calls: &mut Vec<Value>) {
    if pending_tool_calls.is_empty() {
        return;
    }
    messages.push(json!({
        "role": "assistant",
        "content": null,
        "reasoning_content": "tool call",
        "tool_calls": std::mem::take(pending_tool_calls)
    }));
}

fn responses_message_to_chat(item: &Value) -> Value {
    let role = match item.get("role").and_then(Value::as_str).unwrap_or("user") {
        "system" | "developer" => "system",
        "assistant" => "assistant",
        "tool" => "tool",
        _ => "user",
    };
    let content = item
        .get("content")
        .map(responses_content_to_chat_content)
        .unwrap_or(Value::Null);
    json!({ "role": role, "content": content })
}

fn responses_content_to_chat_content(content: &Value) -> Value {
    if content.is_string() || content.is_null() {
        return content.clone();
    }
    let Some(parts) = content.as_array() else {
        return content.clone();
    };

    let mut text_parts = Vec::new();
    let mut rich_parts = Vec::new();
    let mut has_non_text = false;
    for part in parts {
        match part.get("type").and_then(Value::as_str).unwrap_or("") {
            "input_text" | "output_text" | "text" => {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    text_parts.push(text);
                    rich_parts.push(json!({ "type": "text", "text": text }));
                }
            }
            "input_image" => {
                has_non_text = true;
                if let Some(image_url) = part.get("image_url") {
                    let image_url = if image_url.is_object() {
                        image_url.clone()
                    } else {
                        json!({ "url": image_url.as_str().unwrap_or_default() })
                    };
                    rich_parts.push(json!({ "type": "image_url", "image_url": image_url }));
                }
            }
            _ => {}
        }
    }

    if has_non_text {
        Value::Array(rich_parts)
    } else {
        Value::String(text_parts.join("\n"))
    }
}

fn collapse_system_messages(messages: Vec<Value>) -> Vec<Value> {
    let mut system = Vec::new();
    let mut rest = Vec::new();
    for message in messages {
        if message.get("role").and_then(Value::as_str) == Some("system") {
            if let Some(text) = message.get("content").and_then(Value::as_str) {
                if !text.trim().is_empty() {
                    system.push(text.to_string());
                }
            }
        } else {
            rest.push(message);
        }
    }
    if system.is_empty() {
        rest
    } else {
        let mut output = vec![json!({ "role": "system", "content": system.join("\n\n") })];
        output.extend(rest);
        output
    }
}

fn apply_reasoning(result: &mut Value, body: &Value, config: Option<&CodexChatReasoning>) {
    let Some(config) = config else {
        return;
    };
    let Some(enabled) = reasoning_requested(body) else {
        return;
    };
    if config.supports_thinking {
        match config.thinking_param.as_str() {
            "thinking" => {
                result["thinking"] = json!({ "type": if enabled { "enabled" } else { "disabled" } })
            }
            "enable_thinking" => result["enable_thinking"] = json!(enabled),
            "reasoning_split" => result["reasoning_split"] = json!(enabled),
            _ => {}
        }
    }
    if !enabled || !config.supports_effort {
        return;
    }
    let Some(effort) = body.pointer("/reasoning/effort").and_then(Value::as_str) else {
        return;
    };
    let mapped = match config.effort_value_mode.as_str() {
        "deepseek" => {
            if matches!(effort, "max" | "xhigh") {
                "max"
            } else {
                "high"
            }
        }
        _ => effort,
    };
    if config.effort_param == "reasoning_effort" {
        result["reasoning_effort"] = json!(mapped);
    }
}

fn reasoning_requested(body: &Value) -> Option<bool> {
    if let Some(effort) = body.pointer("/reasoning/effort").and_then(Value::as_str) {
        return Some(!matches!(effort, "none" | "off" | "disabled"));
    }
    body.get("reasoning").map(|value| !value.is_null())
}

fn responses_tool_to_chat_tool(tool: &Value) -> Option<Value> {
    if tool.get("type").and_then(Value::as_str) != Some("function") {
        return None;
    }
    if tool.get("function").is_some() {
        return Some(tool.clone());
    }
    Some(json!({
        "type": "function",
        "function": {
            "name": tool.get("name").and_then(Value::as_str).unwrap_or(""),
            "description": tool.get("description").cloned().unwrap_or(Value::Null),
            "parameters": tool.get("parameters").cloned().unwrap_or_else(|| json!({})),
        }
    }))
}

fn responses_tool_choice_to_chat(tool_choice: &Value) -> Value {
    match tool_choice {
        Value::Object(obj) if obj.get("type").and_then(Value::as_str) == Some("function") => {
            json!({
                "type": "function",
                "function": { "name": obj.get("name").and_then(Value::as_str).unwrap_or("") }
            })
        }
        _ => tool_choice.clone(),
    }
}

fn responses_function_call_to_chat(item: &Value) -> Value {
    json!({
        "id": response_item_call_id(item),
        "type": "function",
        "function": {
            "name": item.get("name").and_then(Value::as_str).unwrap_or(""),
            "arguments": canonicalize_arguments(item.get("arguments")),
        }
    })
}

fn response_item_call_id(item: &Value) -> String {
    item.get("call_id")
        .or_else(|| item.get("id"))
        .and_then(Value::as_str)
        .unwrap_or("call_0")
        .to_string()
}

fn chat_reasoning_text(message: &Value) -> Option<String> {
    extract_reasoning_text(message).or_else(|| {
        let content = message.get("content").and_then(Value::as_str)?;
        split_leading_think_block(content).map(|(reasoning, _answer)| reasoning)
    })
}

pub fn extract_reasoning_text(value: &Value) -> Option<String> {
    for key in ["reasoning_content", "reasoning"] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }
    if let Some(reasoning) = value.get("reasoning") {
        for key in ["content", "text", "summary"] {
            if let Some(text) = reasoning.get(key).and_then(Value::as_str) {
                if !text.is_empty() {
                    return Some(text.to_string());
                }
            }
        }
    }
    None
}

fn chat_message_to_response_output_item(message: &Value, response_id: &str) -> Option<Value> {
    let mut content = Vec::new();
    if let Some(text) = message.get("content").and_then(Value::as_str) {
        let text = split_leading_think_block(text)
            .map(|(_reasoning, answer)| answer)
            .unwrap_or_else(|| text.to_string());
        if !text.is_empty() {
            content.push(json!({ "type": "output_text", "text": text, "annotations": [] }));
        }
    }
    if content.is_empty() {
        return None;
    }
    Some(json!({
        "id": format!("{response_id}_msg"),
        "type": "message",
        "status": "completed",
        "role": "assistant",
        "content": content,
    }))
}

fn chat_tool_calls_to_response_items(message: &Value, reasoning: Option<&str>) -> Vec<Value> {
    let mut output = Vec::new();
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for (index, call) in calls.iter().enumerate() {
            let call_id = call
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("call_{index}"));
            let function = call.get("function").unwrap_or(&Value::Null);
            let mut item = json!({
                "id": format!("fc_{call_id}"),
                "type": "function_call",
                "status": "completed",
                "call_id": call_id,
                "name": function.get("name").and_then(Value::as_str).unwrap_or(""),
                "arguments": canonicalize_arguments(function.get("arguments")),
            });
            if let Some(reasoning) = reasoning.filter(|text| !text.trim().is_empty()) {
                item["reasoning_content"] = json!(reasoning);
            }
            output.push(item);
        }
    }
    output
}

fn canonicalize_arguments(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|value| serde_json::to_string(&value).ok())
            .unwrap_or_else(|| text.clone()),
        Some(value) => serde_json::to_string(value).unwrap_or_default(),
        None => String::new(),
    }
}

fn split_leading_think_block(text: &str) -> Option<(String, String)> {
    let trimmed = text.trim_start();
    let body = trimmed.strip_prefix("<think>")?;
    let close = body.find("</think>")?;
    let reasoning = body[..close].trim().to_string();
    let answer = body[close + "</think>".len()..].trim_start().to_string();
    Some((reasoning, answer))
}
