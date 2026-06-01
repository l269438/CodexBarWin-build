use std::{convert::Infallible, net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use bytes::Bytes;
use futures::{Stream, StreamExt};
use serde_json::{Value, json};
use tokio::sync::{RwLock, oneshot};

use crate::{
    history::ConversationHistoryStore,
    store::{ApiFormat, AppConfig, Provider},
    transform::{
        CodexChatReasoning, chat_completion_to_response, chat_usage_to_responses_usage,
        extract_reasoning_text, response_id_from_chat_id, response_status_from_finish_reason,
        responses_to_chat_completions,
    },
};

const STREAM_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Clone)]
struct ProxyRuntime {
    config: Arc<RwLock<AppConfig>>,
    history: Arc<ConversationHistoryStore>,
    client: reqwest::Client,
}

pub struct ProxyHandle {
    stop_tx: Option<oneshot::Sender<()>>,
}

impl ProxyHandle {
    pub fn stop(mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
    }
}

pub async fn start_proxy(
    port: u16,
    config: Arc<RwLock<AppConfig>>,
    history: Arc<ConversationHistoryStore>,
) -> anyhow::Result<ProxyHandle> {
    let runtime = ProxyRuntime {
        config,
        history,
        client: reqwest::Client::new(),
    };
    let app = Router::new()
        .route("/responses", post(handle_responses))
        .route("/v1/responses", post(handle_responses))
        .route("/responses/compact", post(handle_responses))
        .route("/v1/responses/compact", post(handle_responses))
        .with_state(runtime);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let (stop_tx, stop_rx) = oneshot::channel::<()>();

    tokio::spawn(async move {
        let result = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = stop_rx.await;
            })
            .await;
        if let Err(err) = result {
            eprintln!("proxy stopped with error: {err}");
        }
    });

    Ok(ProxyHandle {
        stop_tx: Some(stop_tx),
    })
}

async fn handle_responses(
    State(runtime): State<ProxyRuntime>,
    Json(body): Json<Value>,
) -> Response {
    match forward_codex_request(runtime, body).await {
        Ok(response) => response,
        Err(err) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "error": {
                    "message": err.to_string(),
                    "type": "proxy_error"
                }
            })),
        )
            .into_response(),
    }
}

async fn forward_codex_request(runtime: ProxyRuntime, body: Value) -> anyhow::Result<Response> {
    let provider = {
        let config = runtime.config.read().await;
        config
            .current_provider()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no current provider selected"))?
    };

    match provider.api_format {
        ApiFormat::OpenAiChat => forward_to_chat_provider(runtime, provider, body).await,
        ApiFormat::OpenAiResponses => forward_to_responses_provider(runtime, provider, body).await,
    }
}

async fn forward_to_responses_provider(
    runtime: ProxyRuntime,
    provider: Provider,
    body: Value,
) -> anyhow::Result<Response> {
    let url = responses_url(&provider.base_url);
    let upstream = runtime
        .client
        .post(url)
        .bearer_auth(provider.api_key)
        .json(&body)
        .send()
        .await?;

    let status = upstream.status();
    if !status.is_success() {
        let text = upstream.text().await.unwrap_or_default();
        return Ok((
            StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
            Json(json!({ "error": { "message": text, "type": "upstream_error" } })),
        )
            .into_response());
    }

    let status = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::OK);
    let content_type = upstream
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    let mut builder = Response::builder().status(status);
    if let Some(content_type) = content_type {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }

    if body.get("stream").and_then(Value::as_bool).unwrap_or(false) {
        return Ok(builder.body(Body::from_stream(upstream.bytes_stream()))?);
    }

    Ok(builder.body(Body::from(upstream.bytes().await?))?)
}

async fn forward_to_chat_provider(
    runtime: ProxyRuntime,
    provider: Provider,
    body: Value,
) -> anyhow::Result<Response> {
    let is_stream = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
    let mut request_body = responses_to_chat_completions(
        body.clone(),
        Some(&provider.model),
        Some(&CodexChatReasoning::deepseek()),
    )?;
    runtime
        .history
        .enrich_chat_request(&body, &mut request_body)
        .await;
    let request_messages = request_body
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let url = chat_completions_url(&provider.base_url);
    let request = runtime
        .client
        .post(url)
        .bearer_auth(provider.api_key)
        .json(&request_body);

    if is_stream {
        let stream = chat_provider_sse_stream(
            request,
            runtime.history.clone(),
            request_messages,
            STREAM_KEEPALIVE_INTERVAL,
        );
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, "text/event-stream".parse().unwrap());
        headers.insert(header::CACHE_CONTROL, "no-cache".parse().unwrap());
        return Ok((headers, Body::from_stream(stream)).into_response());
    }

    let upstream = request.send().await?;

    let status = upstream.status();
    if !status.is_success() {
        let text = upstream.text().await.unwrap_or_default();
        return Ok((
            StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
            Json(json!({ "error": { "message": text, "type": "upstream_error" } })),
        )
            .into_response());
    }

    let content_type = upstream
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();

    if is_stream || content_type.contains("text/event-stream") {
        let stream = chat_sse_to_responses_sse_with_keepalive(
            upstream.bytes_stream(),
            runtime.history.clone(),
            request_messages,
            STREAM_KEEPALIVE_INTERVAL,
        );
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, "text/event-stream".parse().unwrap());
        headers.insert(header::CACHE_CONTROL, "no-cache".parse().unwrap());
        return Ok((headers, Body::from_stream(stream)).into_response());
    }

    let chat_response: Value = upstream.json().await?;
    let responses_response = chat_completion_to_response(chat_response.clone())?;
    runtime
        .history
        .record_chat_response(request_messages, &chat_response)
        .await?;
    Ok(Json(responses_response).into_response())
}

pub fn chat_completions_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else if trimmed.ends_with("/v1") {
        format!("{trimmed}/chat/completions")
    } else {
        format!("{trimmed}/v1/chat/completions")
    }
}

pub fn responses_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/responses") {
        trimmed.to_string()
    } else if trimmed.ends_with("/v1") {
        format!("{trimmed}/responses")
    } else {
        format!("{trimmed}/v1/responses")
    }
}

#[derive(Default)]
struct StreamState {
    response_started: bool,
    response_id: String,
    model: String,
    created_at: u64,
    text: String,
    reasoning: String,
    usage: Option<Value>,
    finish_reason: Option<String>,
    tool_calls: Vec<StreamToolCall>,
}

#[derive(Default, Clone)]
struct StreamToolCall {
    id: String,
    name: String,
    arguments: String,
    output_added: bool,
}

fn chat_provider_sse_stream(
    request: reqwest::RequestBuilder,
    history: Arc<ConversationHistoryStore>,
    request_messages: Vec<Value>,
    keepalive_interval: Duration,
) -> impl Stream<Item = Result<Bytes, Infallible>> + Send {
    async_stream::stream! {
        let send = request.send();
        tokio::pin!(send);
        let upstream = loop {
            match tokio::time::timeout(keepalive_interval, send.as_mut()).await {
                Ok(Ok(response)) => break response,
                Ok(Err(err)) => {
                    yield Ok(response_failed_event(&StreamState::default(), err.to_string()));
                    return;
                }
                Err(_) => yield Ok(sse_keepalive()),
            }
        };

        let status = upstream.status();
        if !status.is_success() {
            let read_text = upstream.text();
            tokio::pin!(read_text);
            let message = loop {
                match tokio::time::timeout(keepalive_interval, read_text.as_mut()).await {
                    Ok(Ok(text)) => break text,
                    Ok(Err(err)) => break err.to_string(),
                    Err(_) => yield Ok(sse_keepalive()),
                }
            };
            yield Ok(response_failed_event(&StreamState::default(), message));
            return;
        }

        let responses = chat_sse_to_responses_sse_with_keepalive(
            upstream.bytes_stream(),
            history,
            request_messages,
            keepalive_interval,
        );
        tokio::pin!(responses);
        while let Some(item) = responses.next().await {
            yield item;
        }
    }
}

fn chat_sse_to_responses_sse_with_keepalive(
    stream: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
    history: Arc<ConversationHistoryStore>,
    request_messages: Vec<Value>,
    keepalive_interval: Duration,
) -> impl Stream<Item = Result<Bytes, Infallible>> + Send {
    async_stream::stream! {
        let mut buffer = String::new();
        let mut state = StreamState {
            response_id: "resp_codex_api_switcher".to_string(),
            ..Default::default()
        };
        tokio::pin!(stream);

        loop {
            let item = match tokio::time::timeout(keepalive_interval, stream.next()).await {
                Ok(Some(item)) => item,
                Ok(None) => break,
                Err(_) => {
                    yield Ok(sse_keepalive());
                    continue;
                }
            };
            let Ok(bytes) = item else {
                yield Ok(sse_event("response.failed", json!({
                    "type": "response.failed",
                    "response": base_response(&state, "failed", vec![])
                })));
                return;
            };
            buffer.push_str(&String::from_utf8_lossy(&bytes));
            while let Some(block) = take_sse_block(&mut buffer) {
                let data = sse_data(&block);
                if data.trim().is_empty() {
                    continue;
                }
                if data.trim() == "[DONE]" {
                    history.record_stream_response_with_tool_calls(
                        &state.response_id,
                        request_messages.clone(),
                        &state.text,
                        &state.reasoning,
                        state.chat_tool_calls(),
                    ).await;
                    for event in finalize_stream(&mut state) {
                        yield Ok(event);
                    }
                    return;
                }
                let Ok(chunk) = serde_json::from_str::<Value>(&data) else {
                    continue;
                };
                for event in handle_chat_chunk(&mut state, &chunk) {
                    yield Ok(event);
                }
            }
        }
        history.record_stream_response_with_tool_calls(
            &state.response_id,
            request_messages,
            &state.text,
            &state.reasoning,
            state.chat_tool_calls(),
        ).await;
        for event in finalize_stream(&mut state) {
            yield Ok(event);
        }
    }
}

impl StreamState {
    fn chat_tool_calls(&self) -> Vec<Value> {
        self.tool_calls
            .iter()
            .enumerate()
            .filter(|(_, call)| call.has_content())
            .map(|(index, call)| call.to_chat_tool_call(index))
            .collect()
    }
}

impl StreamToolCall {
    fn has_content(&self) -> bool {
        !self.id.is_empty() || !self.name.is_empty() || !self.arguments.is_empty()
    }

    fn call_id(&self, index: usize) -> String {
        if self.id.is_empty() {
            format!("call_{index}")
        } else {
            self.id.clone()
        }
    }

    fn item_id(&self, index: usize) -> String {
        format!("fc_{}", self.call_id(index))
    }

    fn to_chat_tool_call(&self, index: usize) -> Value {
        json!({
            "id": self.call_id(index),
            "type": "function",
            "function": {
                "name": self.name,
                "arguments": self.arguments,
            }
        })
    }

    fn to_response_item(&self, index: usize, status: &str) -> Value {
        json!({
            "id": self.item_id(index),
            "type": "function_call",
            "status": status,
            "call_id": self.call_id(index),
            "name": self.name,
            "arguments": self.arguments,
        })
    }
}

fn handle_chat_chunk(state: &mut StreamState, chunk: &Value) -> Vec<Bytes> {
    let mut events = Vec::new();
    if let Some(id) = chunk.get("id").and_then(Value::as_str) {
        state.response_id = response_id_from_chat_id(Some(id));
    }
    if let Some(model) = chunk.get("model").and_then(Value::as_str) {
        state.model = model.to_string();
    }
    if let Some(created) = chunk.get("created").and_then(Value::as_u64) {
        state.created_at = created;
    }
    if let Some(usage) = chunk.get("usage").filter(|value| !value.is_null()) {
        state.usage = Some(chat_usage_to_responses_usage(Some(usage)));
    }
    if !state.response_started {
        state.response_started = true;
        events.push(sse_event(
            "response.created",
            json!({
                "type": "response.created",
                "response": base_response(state, "in_progress", vec![])
            }),
        ));
        events.push(sse_event(
            "response.in_progress",
            json!({
                "type": "response.in_progress",
                "response": base_response(state, "in_progress", vec![])
            }),
        ));
    }

    let Some(choice) = chunk
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
    else {
        return events;
    };
    if let Some(delta) = choice.get("delta") {
        if let Some(reasoning) = extract_reasoning_text(delta) {
            if state.reasoning.is_empty() {
                events.push(sse_event(
                    "response.output_item.added",
                    json!({
                        "type": "response.output_item.added",
                        "output_index": 0,
                        "item": {
                            "id": format!("rs_{}", state.response_id),
                            "type": "reasoning",
                            "status": "in_progress",
                            "summary": []
                        }
                    }),
                ));
                events.push(sse_event(
                    "response.reasoning_summary_part.added",
                    json!({
                        "type": "response.reasoning_summary_part.added",
                        "item_id": format!("rs_{}", state.response_id),
                        "output_index": 0,
                        "summary_index": 0,
                        "part": {"type": "summary_text", "text": ""}
                    }),
                ));
            }
            state.reasoning.push_str(&reasoning);
            events.push(sse_event(
                "response.reasoning_summary_text.delta",
                json!({
                    "type": "response.reasoning_summary_text.delta",
                    "item_id": format!("rs_{}", state.response_id),
                    "output_index": 0,
                    "summary_index": 0,
                    "delta": reasoning
                }),
            ));
        }
        if let Some(content) = delta.get("content").and_then(Value::as_str) {
            if !content.is_empty() {
                if state.text.is_empty() {
                    events.push(sse_event(
                        "response.output_item.added",
                        json!({
                            "type": "response.output_item.added",
                            "output_index": if state.reasoning.is_empty() { 0 } else { 1 },
                            "item": {
                                "id": format!("{}_msg", state.response_id),
                                "type": "message",
                                "status": "in_progress",
                                "role": "assistant",
                                "content": []
                            }
                        }),
                    ));
                    events.push(sse_event(
                        "response.content_part.added",
                        json!({
                            "type": "response.content_part.added",
                            "item_id": format!("{}_msg", state.response_id),
                            "output_index": if state.reasoning.is_empty() { 0 } else { 1 },
                            "content_index": 0,
                            "part": {"type": "output_text", "text": "", "annotations": []}
                        }),
                    ));
                }
                state.text.push_str(content);
                events.push(sse_event(
                    "response.output_text.delta",
                    json!({
                        "type": "response.output_text.delta",
                        "item_id": format!("{}_msg", state.response_id),
                        "output_index": if state.reasoning.is_empty() { 0 } else { 1 },
                        "content_index": 0,
                        "delta": content
                    }),
                ));
            }
        }
        if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
            events.extend(handle_chat_tool_call_deltas(state, tool_calls));
        }
    }
    if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
        state.finish_reason = Some(reason.to_string());
    }
    events
}

fn handle_chat_tool_call_deltas(state: &mut StreamState, tool_calls: &[Value]) -> Vec<Bytes> {
    let mut events = Vec::new();
    for (fallback_index, delta) in tool_calls.iter().enumerate() {
        let index = delta
            .get("index")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .unwrap_or(fallback_index);
        while state.tool_calls.len() <= index {
            state.tool_calls.push(StreamToolCall::default());
        }

        let output_index = tool_call_output_index(state, index);
        let call = &mut state.tool_calls[index];
        if let Some(id) = delta
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        {
            call.id = id.to_string();
        }
        if let Some(name) = delta
            .pointer("/function/name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
        {
            call.name = name.to_string();
        }

        if !call.output_added {
            call.output_added = true;
            events.push(sse_event(
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": output_index,
                    "item": call.to_response_item(index, "in_progress")
                }),
            ));
        }

        if let Some(arguments) = delta.pointer("/function/arguments").and_then(Value::as_str) {
            if !arguments.is_empty() {
                call.arguments.push_str(arguments);
                events.push(sse_event(
                    "response.function_call_arguments.delta",
                    json!({
                        "type": "response.function_call_arguments.delta",
                        "item_id": call.item_id(index),
                        "output_index": output_index,
                        "delta": arguments
                    }),
                ));
            }
        }
    }
    events
}

fn tool_call_output_index(state: &StreamState, tool_index: usize) -> usize {
    usize::from(!state.reasoning.is_empty()) + usize::from(!state.text.is_empty()) + tool_index
}

fn finalize_stream(state: &mut StreamState) -> Vec<Bytes> {
    let mut events = Vec::new();
    let mut output = Vec::new();
    if !state.reasoning.is_empty() {
        let item = json!({
            "id": format!("rs_{}", state.response_id),
            "type": "reasoning",
            "summary": [{"type": "summary_text", "text": state.reasoning}]
        });
        output.push(item.clone());
        events.push(sse_event(
            "response.reasoning_summary_text.done",
            json!({
                "type": "response.reasoning_summary_text.done",
                "item_id": format!("rs_{}", state.response_id),
                "output_index": 0,
                "summary_index": 0,
                "text": state.reasoning
            }),
        ));
        events.push(sse_event(
            "response.output_item.done",
            json!({
                "type": "response.output_item.done",
                "output_index": 0,
                "item": item
            }),
        ));
    }
    if !state.text.is_empty() {
        let output_index = if state.reasoning.is_empty() { 0 } else { 1 };
        let item = json!({
            "id": format!("{}_msg", state.response_id),
            "type": "message",
            "status": "completed",
            "role": "assistant",
            "content": [{"type": "output_text", "text": state.text, "annotations": []}]
        });
        output.push(item.clone());
        events.push(sse_event(
            "response.output_text.done",
            json!({
                "type": "response.output_text.done",
                "item_id": format!("{}_msg", state.response_id),
                "output_index": output_index,
                "content_index": 0,
                "text": state.text
            }),
        ));
        events.push(sse_event(
            "response.output_item.done",
            json!({
                "type": "response.output_item.done",
                "output_index": output_index,
                "item": item
            }),
        ));
    }
    for index in 0..state.tool_calls.len() {
        let call = &state.tool_calls[index];
        if !call.has_content() {
            continue;
        }
        let output_index = output.len();
        let item = call.to_response_item(index, "completed");
        events.push(sse_event(
            "response.function_call_arguments.done",
            json!({
                "type": "response.function_call_arguments.done",
                "item_id": call.item_id(index),
                "output_index": output_index,
                "arguments": call.arguments
            }),
        ));
        events.push(sse_event(
            "response.output_item.done",
            json!({
                "type": "response.output_item.done",
                "output_index": output_index,
                "item": item
            }),
        ));
        output.push(item);
    }
    events.push(sse_event(
        "response.completed",
        json!({
            "type": "response.completed",
            "response": base_response(
                state,
                response_status_from_finish_reason(state.finish_reason.as_deref()),
                output,
            )
        }),
    ));
    events
}

fn base_response(state: &StreamState, status: &str, output: Vec<Value>) -> Value {
    json!({
        "id": state.response_id,
        "object": "response",
        "created_at": state.created_at,
        "status": status,
        "model": state.model,
        "output": output,
        "usage": state.usage.clone().unwrap_or_else(|| json!({
            "input_tokens": 0,
            "output_tokens": 0,
            "total_tokens": 0
        }))
    })
}

fn take_sse_block(buffer: &mut String) -> Option<String> {
    if let Some(index) = buffer.find("\n\n") {
        let block = buffer[..index].to_string();
        buffer.drain(..index + 2);
        return Some(block);
    }
    if let Some(index) = buffer.find("\r\n\r\n") {
        let block = buffer[..index].to_string();
        buffer.drain(..index + 4);
        return Some(block);
    }
    None
}

fn sse_data(block: &str) -> String {
    block
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim_start)
        .collect::<Vec<_>>()
        .join("\n")
}

fn sse_event(event: &str, data: Value) -> Bytes {
    Bytes::from(format!(
        "event: {event}\ndata: {}\n\n",
        serde_json::to_string(&data).unwrap_or_default()
    ))
}

fn sse_keepalive() -> Bytes {
    Bytes::from(": codexpilot-keepalive\n\n")
}

fn response_failed_event(state: &StreamState, message: String) -> Bytes {
    sse_event(
        "response.failed",
        json!({
            "type": "response.failed",
            "response": base_response(state, "failed", vec![]),
            "error": {
                "message": message,
                "type": "proxy_error"
            }
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{StreamExt, stream};

    #[tokio::test]
    async fn response_stream_sends_keepalive_while_upstream_is_quiet() {
        let history = Arc::new(ConversationHistoryStore::default());
        let upstream = stream::pending::<Result<Bytes, reqwest::Error>>();
        let mut responses = Box::pin(chat_sse_to_responses_sse_with_keepalive(
            upstream,
            history,
            Vec::new(),
            Duration::from_millis(1),
        ));

        let chunk = tokio::time::timeout(Duration::from_millis(100), responses.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();

        assert_eq!(
            String::from_utf8(chunk.to_vec()).unwrap(),
            ": codexpilot-keepalive\n\n"
        );
    }

    #[tokio::test]
    async fn streamed_chat_tool_call_emits_response_function_call_and_records_history() {
        let history = Arc::new(ConversationHistoryStore::default());
        let request_messages = vec![json!({"role": "user", "content": "read file"})];
        let sse = concat!(
            "data: {\"id\":\"chatcmpl_tool\",\"created\":123,\"model\":\"deepseek-v4-flash\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"read_file\",\"arguments\":\"{\\\"path\\\"\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl_tool\",\"created\":123,\"model\":\"deepseek-v4-flash\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\":\\\"README.md\\\"}\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl_tool\",\"created\":123,\"model\":\"deepseek-v4-flash\",\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let upstream = stream::iter(vec![Ok::<Bytes, reqwest::Error>(Bytes::from(sse))]);

        let chunks = chat_sse_to_responses_sse_with_keepalive(
            upstream,
            history.clone(),
            request_messages,
            Duration::from_secs(10),
        )
        .collect::<Vec<_>>()
        .await;
        let output = chunks
            .into_iter()
            .map(|chunk| String::from_utf8(chunk.unwrap().to_vec()).unwrap())
            .collect::<String>();

        assert!(output.contains("response.function_call_arguments.delta"));
        assert!(output.contains("\"type\":\"function_call\""));
        assert!(output.contains("\"call_id\":\"call_1\""));
        assert!(output.contains("\"name\":\"read_file\""));
        assert!(output.contains("\"arguments\":\"{\\\"path\\\":\\\"README.md\\\"}\""));

        let responses_request = json!({
            "previous_response_id": "resp_chatcmpl_tool",
            "input": [{
                "type": "function_call_output",
                "call_id": "call_1",
                "output": "file text"
            }]
        });
        let mut chat_request = responses_to_chat_completions(
            responses_request.clone(),
            Some("deepseek-v4-flash"),
            None,
        )
        .unwrap();

        history
            .enrich_chat_request(&responses_request, &mut chat_request)
            .await;

        assert_eq!(chat_request["messages"][1]["role"], "assistant");
        assert_eq!(chat_request["messages"][1]["tool_calls"][0]["id"], "call_1");
        assert_eq!(
            chat_request["messages"][1]["tool_calls"][0]["function"]["name"],
            "read_file"
        );
        assert_eq!(chat_request["messages"][2]["role"], "tool");
        assert_eq!(chat_request["messages"][2]["tool_call_id"], "call_1");
    }
}
