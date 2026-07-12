use std::collections::HashMap;
use std::sync::Arc;
use rand::seq::SliceRandom;
use serde_json::{json, Value};
use uuid::Uuid;
use log::{info, error, debug};
use futures_util::{Stream, StreamExt};
use std::pin::Pin;
use std::task::{Context, Poll};
use actix_web::web::Bytes;

use crate::chatgpt::service::ChatService;
use crate::api::tokens::{split_tokens_from_content, num_tokens_from_messages, calculate_image_tokens};

const MODERATION_MESSAGE: &str = "I'm sorry, I cannot provide or engage in any content related to pornography, violence, or any unethical material. If you have any other questions or need assistance, please feel free to let me know. I'll do my best to provide support and assistance.";

// 系统的 system_fingerprint 映射
fn get_system_fingerprint(model: &str) -> Option<String> {
    let mut fingerprints = HashMap::new();
    fingerprints.insert("gpt-3.5-turbo-0125", vec!["fp_b28b39ffa8"]);
    fingerprints.insert("gpt-3.5-turbo-1106", vec!["fp_592ef5907d"]);
    fingerprints.insert("gpt-4-0125-preview", vec!["fp_f38f4d6482", "fp_2f57f81c11", "fp_a7daf7c51e", "fp_a865e8ede4", "fp_13c70b9f70", "fp_b77cb481ed"]);
    fingerprints.insert("gpt-4-1106-preview", vec!["fp_e467c31c3d", "fp_d986a8d1ba", "fp_99a5a401bb", "fp_123d5a9f90", "fp_0d1affc7a6", "fp_5c95a4634e"]);
    fingerprints.insert("gpt-4-turbo-2024-04-09", vec!["fp_d1bac968b4"]);
    fingerprints.insert("gpt-4o-2024-05-13", vec!["fp_3aa7262c27"]);
    fingerprints.insert("gpt-4o-mini-2024-07-18", vec!["fp_c9aa9c0491"]);

    if let Some(list) = fingerprints.get(model) {
        let mut rng = rand::thread_rng();
        list.choose(&mut rng).map(|&s| s.to_string())
    } else {
        None
    }
}

// 辅助从 url 提取文本（上传场景）
fn get_url_from_content(content: &str) -> (Option<String>, String) {
    if content.starts_with("http") {
        if let Some(first_space) = content.find(' ') {
            let url = content[..first_space].trim().to_string();
            let remainder = content[first_space..].trim().to_string();
            return (Some(url), remainder);
        } else {
            return (Some(content.trim().to_string()), String::new());
        }
    }
    (None, content.to_string())
}

pub fn format_messages_with_url(content: &str) -> Value {
    let mut url_list = Vec::new();
    let mut remainder = content.to_string();
    loop {
        let (url, rem) = get_url_from_content(&remainder);
        if let Some(u) = url {
            url_list.push(u);
            remainder = rem;
        } else {
            break;
        }
    }

    if url_list.is_empty() {
        return Value::String(content.to_string());
    }

    let mut content_arr = vec![
        json!({
            "type": "text",
            "text": remainder
        })
    ];

    for url in url_list {
        content_arr.push(json!({
            "type": "image_url",
            "image_url": {
                "url": url
            }
        }));
    }

    Value::Array(content_arr)
}

// 将 OpenAI messages 列表转换为 ChatGPT Web 消息协议格式
pub async fn api_messages_to_chat(
    service: &ChatService,
    api_messages: &Value,
    upload_by_url: bool,
) -> Result<(Value, usize), actix_web::Error> {
    let mut chat_messages = Vec::new();
    let mut file_tokens = 0;

    if let Some(arr) = api_messages.as_array() {
        for msg in arr {
            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            let mut content_val = msg.get("content").cloned().unwrap_or(Value::Null);

            if upload_by_url {
                if let Some(text_str) = content_val.as_str() {
                    content_val = format_messages_with_url(text_str);
                }
            }

            let mut parts = Vec::new();
            let mut attachments = Vec::new();
            let mut content_type = "text";

            if let Some(content_arr) = content_val.as_array() {
                content_type = "multimodal_text";
                for item in content_arr {
                    if let Some(t_type) = item.get("type").and_then(|v| v.as_str()) {
                        if t_type == "text" {
                            if let Some(txt) = item.get("text").and_then(|v| v.as_str()) {
                                parts.push(json!(txt));
                            }
                        } else if t_type == "image_url" {
                            if let Some(img_url_val) = item.get("image_url") {
                                if let Some(url) = img_url_val.get("url").and_then(|v| v.as_str()) {
                                    let detail = img_url_val.get("detail").and_then(|v| v.as_str()).unwrap_or("auto");
                                    
                                    // 模拟文件上传逻辑，获取文件大小和宽高
                                    // 在目前重构版本里，若接口需要多模态文件上传，
                                    // 我们可以请求对应的 URL 并执行上传。
                                    if let Ok((file_content, mime)) = service.get_file_content_from_url(url).await {
                                        if let Ok(Some(file_meta)) = service.upload_file(&file_content, &mime).await {
                                            let file_id = file_meta.file_id;
                                            let file_size = file_meta.size_bytes;
                                            let file_name = file_meta.file_name;
                                            let mime_type = file_meta.mime_type;
                                            let use_case = file_meta.use_case;

                                            if mime_type.starts_with("image/") {
                                                let width = file_meta.width.unwrap_or(512);
                                                let height = file_meta.height.unwrap_or(512);
                                                file_tokens += calculate_image_tokens(width, height, detail) as usize;
                                                parts.push(json!({
                                                    "content_type": "image_asset_pointer",
                                                    "asset_pointer": format!("file-service://{}", file_id),
                                                    "size_bytes": file_size,
                                                    "width": width,
                                                    "height": height
                                                }));
                                                attachments.push(json!({
                                                    "id": file_id,
                                                    "size": file_size,
                                                    "name": file_name,
                                                    "mime_type": mime_type,
                                                    "width": width,
                                                    "height": height
                                                }));
                                            } else {
                                                if use_case != "ace_upload" {
                                                    service.check_upload(&file_id).await;
                                                }
                                                file_tokens += file_size / 1000;
                                                attachments.push(json!({
                                                    "id": file_id,
                                                    "size": file_size,
                                                    "name": file_name,
                                                    "mime_type": mime_type,
                                                }));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else if let Some(text_str) = content_val.as_str() {
                parts.push(json!(text_str));
            }

            let metadata = if attachments.is_empty() {
                json!({})
            } else {
                json!({ "attachments": attachments })
            };

            let chat_msg = json!({
                "id": Uuid::new_v4().to_string(),
                "author": { "role": role },
                "content": { "content_type": content_type, "parts": parts },
                "metadata": metadata
            });
            chat_messages.push(chat_msg);
        }
    }

    let text_tokens = num_tokens_from_messages(api_messages, &service.resp_model);
    let prompt_tokens = text_tokens + file_tokens;

    Ok((Value::Array(chat_messages), prompt_tokens))
}

// 辅助检测响应中的第一行数据包状态
pub async fn head_process_response<S>(mut stream: S) -> (S, bool)
where
    S: Stream<Item = Result<Bytes, rquest::Error>> + Unpin,
{
    // 在这里我们可能需要读取流的前几行来进行验证。
    // 为了简化流式处理在 Service 的逻辑，我们在 Service 发送完请求后，
    // 读取第一行 data 并判断是否有 status == "in_progress" 或包含错误信息。
    // 在 Rust 里一般建议直接在 stream 处理里进行。
    (stream, true)
}

// 流式 SSE 的包装转换器
pub struct OpenAIStream {
    service: Arc<ChatService>,
    raw_stream: Pin<Box<dyn Stream<Item = Result<Bytes, rquest::Error>> + Send>>,
    chat_id: String,
    created_time: i64,
    model: String,
    system_fingerprint: Option<String>,
    
    // 转换状态累加器
    completion_tokens: usize,
    max_tokens: usize,
    len_last_content: usize,
    len_last_citation: usize,
    last_message_id: Option<String>,
    last_role: Option<String>,
    last_content_type: Option<String>,
    last_status: Option<String>,
    model_slug: Option<String>,
    end: bool,
    buffer: String,
}

impl OpenAIStream {
    pub fn new(
        service: Arc<ChatService>,
        raw_stream: Pin<Box<dyn Stream<Item = Result<Bytes, rquest::Error>> + Send>>,
        model: String,
        max_tokens: usize,
    ) -> Self {
        let mut rng = rand::thread_rng();
        let chat_id = format!("chatcmpl-{}", Uuid::new_v4().to_string().replace("-", "")[..29].to_string());
        let system_fingerprint = get_system_fingerprint(&model);
        let created_time = chrono::Utc::now().timestamp();

        Self {
            service,
            raw_stream,
            chat_id,
            created_time,
            model,
            system_fingerprint,
            completion_tokens: 0,
            max_tokens,
            len_last_content: 0,
            len_last_citation: 0,
            last_message_id: None,
            last_role: None,
            last_content_type: None,
            last_status: None,
            model_slug: None,
            end: false,
            buffer: String::new(),
        }
    }
}

// 实现 Stream 特性，使 OpenAIStream 可以被 Actix-web 优雅地进行 SSE 代理
impl Stream for OpenAIStream {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.end {
            return Poll::Ready(None);
        }

        // 第一帧：发送 role 和初始 content
        if self.completion_tokens == 0 {
            self.completion_tokens += 1;
            let mut first_delta = json!({
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
            if let Some(ref fp) = self.system_fingerprint {
                first_delta.as_object_mut().unwrap().insert("system_fingerprint".to_string(), json!(fp));
            }
            let bytes_payload = Bytes::from(format!("data: {}\n\n", first_delta.to_string()));
            return Poll::Ready(Some(Ok(bytes_payload)));
        }

        // 不断拉取原始流并转换
        match Pin::new(&mut self.raw_stream).poll_next(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(None) => {
                self.end = true;
                return Poll::Ready(Some(Ok(Bytes::from("data: [DONE]\n\n"))));
            }
            Poll::Ready(Some(Err(e))) => {
                self.end = true;
                return Poll::Ready(Some(Err(std::io::Error::new(std::io::ErrorKind::Other, e))));
            }
            Poll::Ready(Some(Ok(bytes))) => {
                let chunk_str = String::from_utf8_lossy(&bytes).to_string();
                self.buffer.push_str(&chunk_str);
                let mut output_bytes = Vec::new();

                while let Some(pos) = self.buffer.find('\n') {
                    let line = self.buffer[..pos].to_string();
                    self.buffer = self.buffer[pos + 1..].to_string();
                    
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    if line.starts_with("data: [DONE]") {
                        self.end = true;
                        output_bytes.extend_from_slice(b"data: [DONE]\n\n");
                        break;
                    }
                    if !line.starts_with("data: {") {
                        continue;
                    }
                    // info!("OpenAI chunk line: {}", line);
                    let json_str = &line[6..];
                    let old_data: Value = match serde_json::from_str(json_str) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };

                    let message = old_data.get("message").cloned().unwrap_or(Value::Null);
                    let author = message.get("author").cloned().unwrap_or(Value::Null);
                    let role = author.get("role").and_then(|v| v.as_str()).unwrap_or("");
                    if role == "user" || role == "system" {
                        continue;
                    }

                    let status = message.get("status").and_then(|v| v.as_str()).unwrap_or("");
                    let message_id = message.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
                    let content = message.get("content").cloned().unwrap_or(Value::Null);
                    let metadata = message.get("metadata").cloned().unwrap_or(Value::Null);
                    let initial_text = metadata.get("initial_text").and_then(|v| v.as_str()).unwrap_or("");
                    let recipient = message.get("recipient").and_then(|v| v.as_str()).unwrap_or("");
                    
                    if let Some(slug) = metadata.get("model_slug").and_then(|v| v.as_str()) {
                        self.model_slug = Some(slug.to_string());
                    }

                    let mut finish_reason = Value::Null;
                    let mut delta = json!({});

                    if message.is_null() && old_data.get("type").and_then(|v| v.as_str()) == Some("moderation") {
                        delta = json!({ "role": "assistant", "content": MODERATION_MESSAGE });
                        finish_reason = json!("stop");
                        self.end = true;
                    } else if status == "in_progress" {
                        let outer_content_type = content.get("content_type").and_then(|v| v.as_str()).unwrap_or("");
                        let mut new_text = String::new();

                        if outer_content_type == "text" {
                            let part = content.get("parts").and_then(|v| v.as_array())
                                .and_then(|a| a.first())
                                .and_then(|v| v.as_str())
                                .unwrap_or("");

                            if part.is_empty() {
                                if role == "assistant" && self.last_role.as_deref() != Some("assistant") {
                                    if self.last_role.is_none() {
                                        new_text = "".to_string();
                                    } else {
                                        new_text = "\n".to_string();
                                    }
                                } else if role == "tool" && self.last_role.as_deref() != Some("tool") {
                                    new_text = format!(">{}\n", initial_text);
                                }
                            } else {
                                if self.last_message_id.is_some() && self.last_message_id != message_id {
                                    continue;
                                }
                                
                                let citations = metadata.get("citations").and_then(|v| v.as_array());
                                let citations_len = citations.map(|a| a.len()).unwrap_or(0);

                                if citations_len > self.len_last_citation {
                                    if let Some(cite) = citations.unwrap().get(self.len_last_citation) {
                                        let cite_meta = cite.get("metadata").cloned().unwrap_or(Value::Null);
                                        let title = cite_meta.get("title").and_then(|v| v.as_str()).unwrap_or("");
                                        let url = cite_meta.get("url").and_then(|v| v.as_str()).unwrap_or("");
                                        new_text = format!(" **[[\"\"]]({} \"{}\")** ", url, title);
                                    }
                                    self.len_last_citation = citations_len;
                                } else {
                                    let slice_start = self.len_last_content.min(part.len());
                                    let incoming_text = &part[slice_start..];

                                    if role == "assistant" && self.last_role.as_deref() != Some("assistant") {
                                        if recipient == "dalle.text2im" {
                                            new_text = format!("\n```{}\n{}", recipient, incoming_text);
                                        } else if recipient == "t2uay3k.sj1i4kz" {
                                            new_text = format!("\n```image_creator\n{}", incoming_text);
                                        } else if self.last_role.is_none() {
                                            new_text = incoming_text.to_string();
                                        } else {
                                            new_text = format!("\n\n{}", incoming_text);
                                        }
                                    } else if role == "tool" && self.last_role.as_deref() != Some("tool") {
                                        new_text = format!(">{}\n{}", initial_text, incoming_text);
                                    } else if role == "tool" {
                                        new_text = incoming_text.replace("\n\n", "\n");
                                    } else {
                                        new_text = incoming_text.to_string();
                                    }
                                }
                                self.len_last_content = part.len();
                            }
                        } else if outer_content_type == "multimodal_text" {
                            // 处理图片上传中的多模态接收
                            // 这里可以通过 service 获取图片下载 URL 并输出为 Markdown
                        } else {
                            let text = content.get("text").and_then(|v| v.as_str()).unwrap_or("");
                            let slice_start = self.len_last_content.min(text.len());
                            let incoming_text = &text[slice_start..];

                            if outer_content_type == "code" && self.last_content_type.as_deref() != Some("code") {
                                let mut lang = content.get("language").and_then(|v| v.as_str()).unwrap_or("");
                                if lang.is_empty() || lang == "unknown" {
                                    lang = recipient;
                                }
                                new_text = format!("\n```{}\n{}", lang, incoming_text);
                            } else if outer_content_type == "execution_output" && self.last_content_type.as_deref() != Some("execution_output") {
                                new_text = format!("\n```Output\n{}", incoming_text);
                            } else {
                                new_text = incoming_text.to_string();
                            }
                            self.len_last_content = text.len();
                        }

                        if self.last_content_type.as_deref() == Some("code") && outer_content_type != "code" {
                            new_text = format!("\n```\n{}", new_text);
                        } else if self.last_content_type.as_deref() == Some("execution_output") && outer_content_type != "execution_output" {
                            new_text = format!("\n```\n{}", new_text);
                        }

                        delta = json!({ "content": new_text });
                        self.last_content_type = Some(outer_content_type.to_string());

                        if self.completion_tokens >= self.max_tokens {
                            delta = json!({});
                            finish_reason = json!("length");
                            self.end = true;
                        }
                    } else if status == "finished_successfully" {
                        if message.get("end_turn").and_then(|v| v.as_bool()).unwrap_or(false) {
                            if let Some(parts_arr) = content.get("parts").and_then(|v| v.as_array()) {
                                if let Some(part_str) = parts_arr.first().and_then(|v| v.as_str()) {
                                    let slice_start = self.len_last_content.min(part_str.len());
                                    delta = json!({ "content": &part_str[slice_start..] });
                                }
                            }
                            finish_reason = json!("stop");
                            self.end = true;
                        } else {
                            self.len_last_content = 0;
                            if let Some(finished_txt) = metadata.get("finished_text").and_then(|v| v.as_str()) {
                                delta = json!({ "content": format!("\n{}\n", finished_txt) });
                            } else {
                                continue;
                            }
                        }
                    }

                    self.last_message_id = message_id.clone();
                    self.last_role = Some(role.to_string());
                    self.last_status = Some(status.to_string());

                    if !self.end && delta.get("content").is_none() {
                        delta = json!({ "role": "assistant", "content": "" });
                    }

                    let mut chunk_new_data = json!({
                        "id": self.chat_id,
                        "object": "chat.completion.chunk",
                        "created": self.created_time,
                        "model": self.model,
                        "choices": [{
                            "index": 0,
                            "delta": delta,
                            "logprobs": null,
                            "finish_reason": finish_reason
                        }]
                    });

                    if let Some(ref fp) = self.system_fingerprint {
                        chunk_new_data.as_object_mut().unwrap().insert("system_fingerprint".to_string(), json!(fp));
                    }
                    if !self.service.history_disabled {
                        if let Some(ref mid) = message_id {
                            chunk_new_data.as_object_mut().unwrap().insert("message_id".to_string(), json!(mid));
                        }
                        let conv_id = old_data.get("conversation_id").cloned().unwrap_or(Value::Null);
                        chunk_new_data.as_object_mut().unwrap().insert("conversation_id".to_string(), conv_id);
                    }

                    self.completion_tokens += 1;
                    output_bytes.extend_from_slice(format!("data: {}\n\n", chunk_new_data.to_string()).as_bytes());
                }

                if !output_bytes.is_empty() {
                    return Poll::Ready(Some(Ok(Bytes::from(output_bytes))));
                }
            }
        }

        Poll::Pending
    }
}

// 处理非流式响应转换：合并流中所有的文本
pub async fn format_not_stream_response(
    mut stream: OpenAIStream,
    prompt_tokens: usize,
    max_tokens: usize,
    model: String,
) -> Result<Value, actix_web::Error> {
    let chat_id = stream.chat_id.clone();
    let system_fingerprint = stream.system_fingerprint.clone();
    let created_time = stream.created_time;
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
            let data_json: Value = serde_json::from_str(&line[6..]).unwrap_or(Value::Null);
            if let Some(choices) = data_json.get("choices").and_then(|v| v.as_array()) {
                if let Some(first_choice) = choices.first() {
                    if let Some(content) = first_choice.get("delta").and_then(|d| d.get("content")).and_then(|c| c.as_str()) {
                        all_text.push_str(content);
                    }
                }
            }
        }
    }

    let (content, completion_tokens, finish_reason) = split_tokens_from_content(&all_text, max_tokens, &model);
    
    if content.is_empty() {
        return Err(actix_web::error::ErrorForbidden("No content in the message."));
    }

    let mut response_json = json!({
        "id": chat_id,
        "object": "chat.completion",
        "created": created_time,
        "model": model,
        "choices": [
            {
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": content
                },
                "logprobs": null,
                "finish_reason": finish_reason
            }
        ],
        "usage": {
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": prompt_tokens + completion_tokens
        }
    });

    if let Some(fp) = system_fingerprint {
        response_json.as_object_mut().unwrap().insert("system_fingerprint".to_string(), json!(fp));
    }

    Ok(response_json)
}
