use std::collections::{HashMap, VecDeque};

use serde_json::{Value, json};
use tokio::sync::RwLock;

use crate::transform::response_id_from_chat_id;

const MAX_CACHED_RESPONSES: usize = 512;

#[derive(Debug, Clone, Default)]
struct CachedConversation {
    messages: Vec<Value>,
}

#[derive(Debug, Default)]
struct ConversationHistoryInner {
    responses: HashMap<String, CachedConversation>,
    response_order: VecDeque<String>,
}

#[derive(Debug, Default)]
pub struct ConversationHistoryStore {
    inner: RwLock<ConversationHistoryInner>,
}

impl ConversationHistoryStore {
    pub async fn enrich_chat_request(
        &self,
        responses_body: &Value,
        chat_request: &mut Value,
    ) -> usize {
        let Some(previous_response_id) = responses_body
            .get("previous_response_id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        else {
            return 0;
        };

        let cached = {
            let inner = self.inner.read().await;
            inner.responses.get(previous_response_id).cloned()
        };
        let Some(cached) = cached else {
            return 0;
        };

        let Some(current_messages) = chat_request
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
        else {
            return 0;
        };

        let merged = merge_cached_messages(&cached.messages, &current_messages);
        let added = merged.len().saturating_sub(current_messages.len());
        chat_request["messages"] = Value::Array(merged);
        added
    }

    pub async fn record_chat_response(
        &self,
        request_messages: Vec<Value>,
        chat_response: &Value,
    ) -> anyhow::Result<Option<String>> {
        let Some(message) = assistant_message_from_chat_response(chat_response) else {
            return Ok(None);
        };
        let response_id = response_id_from_chat_id(chat_response.get("id").and_then(Value::as_str));
        self.record_messages(response_id.clone(), request_messages, message)
            .await;
        Ok(Some(response_id))
    }

    pub async fn record_stream_response(
        &self,
        response_id: &str,
        request_messages: Vec<Value>,
        text: &str,
        reasoning: &str,
    ) -> Option<String> {
        if text.is_empty() && reasoning.is_empty() {
            return None;
        }
        let mut message = json!({
            "role": "assistant",
            "content": text,
        });
        if !reasoning.is_empty() {
            message["reasoning_content"] = json!(reasoning);
        }
        self.record_messages(response_id.to_string(), request_messages, message)
            .await;
        Some(response_id.to_string())
    }

    async fn record_messages(
        &self,
        response_id: String,
        request_messages: Vec<Value>,
        assistant_message: Value,
    ) {
        let mut transcript = request_messages
            .into_iter()
            .filter(valid_chat_message)
            .collect::<Vec<_>>();
        transcript.push(normalize_assistant_message(assistant_message));

        let mut inner = self.inner.write().await;
        if !inner.responses.contains_key(&response_id) {
            inner.response_order.push_back(response_id.clone());
        }
        inner.responses.insert(
            response_id,
            CachedConversation {
                messages: transcript,
            },
        );
        inner.prune();
    }
}

impl ConversationHistoryInner {
    fn prune(&mut self) {
        while self.response_order.len() > MAX_CACHED_RESPONSES {
            let Some(response_id) = self.response_order.pop_front() else {
                break;
            };
            self.responses.remove(&response_id);
        }
    }
}

fn merge_cached_messages(cached: &[Value], current: &[Value]) -> Vec<Value> {
    let cached_system = cached.iter().filter(|message| is_system_message(message));
    let current_system = current.iter().filter(|message| is_system_message(message));
    let current_has_system = current.iter().any(is_system_message);

    let mut merged = Vec::new();
    if current_has_system {
        merged.extend(current_system.cloned());
    } else {
        merged.extend(cached_system.cloned());
    }
    merged.extend(
        cached
            .iter()
            .filter(|message| !is_system_message(message))
            .cloned(),
    );
    merged.extend(
        current
            .iter()
            .filter(|message| !is_system_message(message))
            .cloned(),
    );
    merged
}

fn assistant_message_from_chat_response(response: &Value) -> Option<Value> {
    response
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .cloned()
        .map(normalize_assistant_message)
}

fn normalize_assistant_message(mut message: Value) -> Value {
    if !message.is_object() {
        return json!({ "role": "assistant", "content": "" });
    }
    if message.get("role").and_then(Value::as_str).is_none() {
        message["role"] = json!("assistant");
    }
    if message.get("content").is_none() {
        message["content"] = Value::Null;
    }
    if message.get("tool_calls").is_none() {
        if let Some(object) = message.as_object_mut() {
            object.remove("reasoning_content");
            object.remove("reasoning");
        }
    }
    message
}

fn valid_chat_message(message: &Value) -> bool {
    message.get("role").and_then(Value::as_str).is_some()
}

fn is_system_message(message: &Value) -> bool {
    message.get("role").and_then(Value::as_str) == Some("system")
}
