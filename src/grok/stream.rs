use std::pin::Pin;
use std::task::{Context, Poll};
use futures_util::{Stream, StreamExt};
use actix_web::web::Bytes;
use serde_json::{json, Value};
use uuid::Uuid;
use log::error;

pub struct GrokStream {
    pub raw_stream: Pin<Box<dyn Stream<Item = Result<Bytes, rquest::Error>> + Send>>,
    pub chat_id: String,
    pub created_time: i64,
    pub model: String,
    pub end: bool,
    pub buffer: String,
    pub current_event: String,
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub first_chunk_sent: bool,
}

impl GrokStream {
    pub fn new(
        raw_stream: Pin<Box<dyn Stream<Item = Result<Bytes, rquest::Error>> + Send>>,
        model: String,
    ) -> Self {
        let chat_id = format!(
            "chatcmpl-{}",
            &Uuid::new_v4().to_string().replace('-', "")[..29]
        );
        let created_time = chrono::Utc::now().timestamp();
        Self {
            raw_stream,
            chat_id,
            created_time,
            model,
            end: false,
            buffer: String::new(),
            current_event: String::new(),
            input_tokens: 0,
            output_tokens: 0,
            first_chunk_sent: false,
        }
    }
}

fn classify_console_line(line: &str) -> (&str, &str) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return ("skip", "");
    }
    if trimmed.starts_with("event:") {
        return ("event", trimmed[6..].trim());
    }
    if trimmed.starts_with("data:") {
        let data = trimmed[5..].trim();
        if data == "[DONE]" {
            return ("done", "");
        }
        return ("data", data);
    }
    ("skip", "")
}

impl Stream for GrokStream {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.end {
            return Poll::Ready(None);
        }

        if !self.first_chunk_sent {
            self.first_chunk_sent = true;
            let first_delta = json!({
                "id": self.chat_id,
                "object": "chat.completion.chunk",
                "created": self.created_time,
                "model": self.model,
                "choices": [{
                    "index": 0,
                    "delta": { "role": "assistant", "content": "" },
                    "logprobs": null,
                    "finish_reason": null
                }]
            });
            return Poll::Ready(Some(Ok(Bytes::from(format!("data: {}\n\n", first_delta)))));
        }

        loop {
            match Pin::new(&mut self.raw_stream).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => {
                    self.end = true;
                    // Send final chunk with usage
                    let mut final_choice = json!({
                        "id": self.chat_id,
                        "object": "chat.completion.chunk",
                        "created": self.created_time,
                        "model": self.model,
                        "choices": [{
                            "index": 0,
                            "delta": {},
                            "logprobs": null,
                            "finish_reason": "stop"
                        }]
                    });
                    if self.input_tokens > 0 || self.output_tokens > 0 {
                        final_choice.as_object_mut().unwrap().insert(
                            "usage".to_string(),
                            json!({
                                "prompt_tokens": self.input_tokens,
                                "completion_tokens": self.output_tokens,
                                "total_tokens": self.input_tokens + self.output_tokens
                            }),
                        );
                    }
                    let mut output = format!("data: {}\n\n", final_choice);
                    output.push_str("data: [DONE]\n\n");
                    return Poll::Ready(Some(Ok(Bytes::from(output))));
                }
                Poll::Ready(Some(Err(e))) => {
                    error!("Error reading from Grok raw_stream: {:?}", e);
                    self.end = true;
                    return Poll::Ready(Some(Err(std::io::Error::new(std::io::ErrorKind::Other, e))));
                }
                Poll::Ready(Some(Ok(bytes))) => {
                    let chunk_str = String::from_utf8_lossy(&bytes).to_string();
                    self.buffer.push_str(&chunk_str);
                    let mut output_bytes = Vec::new();

                    while let Some(pos) = self.buffer.find('\n') {
                        let line = self.buffer[..pos].trim().to_string();
                        self.buffer = self.buffer[pos + 1..].to_string();

                        if line.is_empty() {
                            continue;
                        }

                        let (kind, value) = classify_console_line(&line);
                        if kind == "event" {
                            self.current_event = value.to_string();
                        } else if kind == "data" {
                            if self.current_event == "response.output_text.delta" {
                                if let Ok(obj) = serde_json::from_str::<Value>(value) {
                                    if let Some(delta_text) = obj.get("delta").and_then(|v| v.as_str()) {
                                        let chunk = json!({
                                            "id": self.chat_id,
                                            "object": "chat.completion.chunk",
                                            "created": self.created_time,
                                            "model": self.model,
                                            "choices": [{
                                                "index": 0,
                                                "delta": { "content": delta_text },
                                                "logprobs": null,
                                                "finish_reason": null
                                            }]
                                        });
                                        output_bytes.extend_from_slice(format!("data: {}\n\n", chunk).as_bytes());
                                    }
                                }
                            } else if self.current_event == "response.completed" {
                                if let Ok(obj) = serde_json::from_str::<Value>(value) {
                                    if let Some(usage) = obj.get("response").and_then(|r| r.get("usage")) {
                                        self.input_tokens = usage.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                                        self.output_tokens = usage.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                                    }
                                }
                            } else if self.current_event == "error" {
                                if let Ok(obj) = serde_json::from_str::<Value>(value) {
                                    let msg = obj.get("message").and_then(|v| v.as_str()).unwrap_or("Unknown console API error");
                                    error!("Grok console stream error: {}", msg);
                                    self.end = true;
                                    output_bytes.extend_from_slice(b"data: [DONE]\n\n");
                                    break;
                                }
                            }
                            self.current_event.clear();
                        } else if kind == "done" {
                            self.end = true;
                            let mut final_choice = json!({
                                "id": self.chat_id,
                                "object": "chat.completion.chunk",
                                "created": self.created_time,
                                "model": self.model,
                                "choices": [{
                                    "index": 0,
                                    "delta": {},
                                    "logprobs": null,
                                    "finish_reason": "stop"
                                }]
                            });
                            if self.input_tokens > 0 || self.output_tokens > 0 {
                                final_choice.as_object_mut().unwrap().insert(
                                    "usage".to_string(),
                                    json!({
                                        "prompt_tokens": self.input_tokens,
                                        "completion_tokens": self.output_tokens,
                                        "total_tokens": self.input_tokens + self.output_tokens
                                    }),
                                );
                            }
                            output_bytes.extend_from_slice(format!("data: {}\n\n", final_choice).as_bytes());
                            output_bytes.extend_from_slice(b"data: [DONE]\n\n");
                            break;
                        }
                    }

                    if !output_bytes.is_empty() {
                        return Poll::Ready(Some(Ok(Bytes::from(output_bytes))));
                    }
                }
            }
        }
    }
}

pub async fn format_not_stream_response(
    mut stream: GrokStream,
) -> Result<Value, actix_web::Error> {
    let chat_id = stream.chat_id.clone();
    let created_time = stream.created_time;
    let model = stream.model.clone();
    let mut all_text = String::new();

    while let Some(chunk_res) = stream.next().await {
        let bytes = chunk_res?;
        let text_chunk = String::from_utf8_lossy(&bytes).to_string();
        for line in text_chunk.lines() {
            let line = line.trim();
            if line.starts_with("data: [DONE]") {
                break;
            }
            if !line.starts_with("data: {") {
                continue;
            }
            let data_json: Value = match serde_json::from_str(&line[6..]) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if let Some(choices) = data_json.get("choices").and_then(|v| v.as_array()) {
                if let Some(first_choice) = choices.first() {
                    let delta = match first_choice.get("delta") {
                        Some(d) if !d.is_null() => d,
                        _ => continue,
                    };
                    if let Some(content_val) = delta.get("content") {
                        if let Some(content_str) = content_val.as_str() {
                            all_text.push_str(content_str);
                        }
                    }
                }
            }
        }
    }

    let input_tokens = stream.input_tokens;
    let output_tokens = stream.output_tokens;

    let response_json = json!({
        "id": chat_id,
        "object": "chat.completion",
        "created": created_time,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": all_text
            },
            "logprobs": null,
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": input_tokens,
            "completion_tokens": output_tokens,
            "total_tokens": input_tokens + output_tokens
        }
    });

    Ok(response_json)
}
