// 本文件主要用于处理 OpenAI 接口请求/响应与 ChatGPT 官方格式的相互转换
// 包含流式 Event-Stream 的正则解析、提取代码块/生成的图片文件、计算 Token 及进行答复重组拼接。
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
use regex::Regex;

use crate::chatgpt::service::ChatService;
use crate::api::tokens::{split_tokens_from_content, num_tokens_from_messages, calculate_image_tokens};

const MODERATION_MESSAGE: &str = "I'm sorry, I cannot provide or engage in any content related to pornography, violence, or any unethical material. If you have any other questions or need assistance, please feel free to let me know. I'll do my best to provide support and assistance.";

fn get_system_fingerprint(model: &str) -> Option<String> {
    let mut fingerprints: HashMap<&str, Vec<&str>> = HashMap::new();
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
    let mut content_arr = vec![json!({ "type": "text", "text": remainder })];
    for url in url_list {
        content_arr.push(json!({ "type": "image_url", "image_url": { "url": url } }));
    }
    Value::Array(content_arr)
}

/// 将客户端标准 OpenAI 格式的 messages 列表，转换为 ChatGPT 官网后端的多模态消息协议格式
/// service: ChatService 实例（包含网络文件上传方法）
/// api_messages: 原始 JSON 消息数组
/// upload_by_url: 是否开启根据内容中的图片 URL 进行自动多模态识别上传
pub async fn api_messages_to_chat(
    service: &ChatService,
    api_messages: &Value,
    upload_by_url: bool,
) -> Result<(Value, usize), actix_web::Error> {
    let mut chat_messages = Vec::new();
    let mut file_tokens = 0; // 累计上传文件的消耗 Token 预算

    if let Some(arr) = api_messages.as_array() {
        for msg in arr {
            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            let mut content_val = msg.get("content").cloned().unwrap_or(Value::Null);

            // 如果配置了通过文本里携带图片 URL 直接提取上传
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
                                                    "mime_type": mime_type
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

            chat_messages.push(json!({
                "id": Uuid::new_v4().to_string(),
                "author": { "role": role },
                "content": { "content_type": content_type, "parts": parts },
                "metadata": metadata
            }));
        }
    }

    let text_tokens = num_tokens_from_messages(api_messages, &service.resp_model);
    let prompt_tokens = text_tokens + file_tokens;
    Ok((Value::Array(chat_messages), prompt_tokens))
}

// 流式 SSE 包装转换器
pub struct OpenAIStream {
    pub service: Arc<ChatService>,
    pub raw_stream: Pin<Box<dyn Stream<Item = Result<Bytes, rquest::Error>> + Send>>,
    pub chat_id: String,
    pub created_time: i64,
    pub model: String,
    pub system_fingerprint: Option<String>,

    pub completion_tokens: usize,
    pub max_tokens: usize,
    pub len_last_content: usize,
    pub len_last_citation: usize,
    pub last_message_id: Option<String>,
    pub last_role: Option<String>,
    pub last_content_type: Option<String>,
    pub last_status: Option<String>,
    pub model_slug: Option<String>,
    pub end: bool,
    pub buffer: String,
}

impl OpenAIStream {
    pub fn new(
        service: Arc<ChatService>,
        raw_stream: Pin<Box<dyn Stream<Item = Result<Bytes, rquest::Error>> + Send>>,
        model: String,
        max_tokens: usize,
    ) -> Self {
        let chat_id = format!(
            "chatcmpl-{}",
            &Uuid::new_v4().to_string().replace('-', "")[..29]
        );
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

impl Stream for OpenAIStream {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.end {
            return Poll::Ready(None);
        }

        // 第一帧：发送 role 初始 chunk
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
            return Poll::Ready(Some(Ok(Bytes::from(format!("data: {}\n\n", first_delta)))));
        }

        loop {
            match Pin::new(&mut self.raw_stream).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => {
                    self.end = true;
                    return Poll::Ready(Some(Ok(Bytes::from("data: [DONE]\n\n"))));
                }
                Poll::Ready(Some(Err(e))) => {
                    error!("Error reading from raw_stream: {:?}", e);
                    self.end = true;
                    return Poll::Ready(Some(Err(std::io::Error::new(std::io::ErrorKind::Other, e))));
                }
                Poll::Ready(Some(Ok(bytes))) => {
                    let chunk_str = String::from_utf8_lossy(&bytes).to_string();
                    self.buffer.push_str(&chunk_str);
                    let mut output_bytes: Vec<u8> = Vec::new();

                    // 处理所有完整行
                    while let Some(pos) = self.buffer.find('\n') {
                        let line = self.buffer[..pos].trim().to_string();
                        self.buffer = self.buffer[pos + 1..].to_string();

                        if line.is_empty() {
                            continue;
                        }
                        if line.starts_with("data: [DONE]") {
                            info!("Response Model: {:?}", self.model_slug);
                            self.end = true;
                            output_bytes.extend_from_slice(b"data: [DONE]\n\n");
                            break;
                        }
                        if !line.starts_with("data: {") {
                            continue;
                        }

                        info!("OpenAI chunk line: {}", line);
                        let json_str = &line[6..];
                        let old_data: Value = match serde_json::from_str(json_str) {
                            Ok(v) => v,
                            Err(_) => continue,
                        };

                        // 检查是否有错误
                        if let Some(err_val) = old_data.get("error") {
                            if !err_val.is_null() {
                                error!("Error from stream: {:?}", err_val);
                                output_bytes.extend_from_slice(b"data: [DONE]\n\n");
                                self.end = true;
                                break;
                            }
                        }

                        let message = old_data.get("message").cloned().unwrap_or(Value::Null);
                        let conversation_id = old_data.get("conversation_id").cloned().unwrap_or(Value::Null);

                        let role = message
                            .get("author")
                            .and_then(|v| v.get("role"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
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

                        // 处理 moderation 事件
                        if message.is_null() && old_data.get("type").and_then(|v| v.as_str()) == Some("moderation") {
                            let delta = json!({ "role": "assistant", "content": MODERATION_MESSAGE });
                            let chunk = build_chunk(&self.chat_id, self.created_time, &self.model, &self.system_fingerprint, delta, json!("stop"), &message_id, &conversation_id, self.service.history_disabled);
                            output_bytes.extend_from_slice(format!("data: {}\n\n", chunk).as_bytes());
                            self.completion_tokens += 1;
                            self.end = true;
                            break;
                        }

                        let finish_reason: Value;
                        let delta: Value;

                        // ── in_progress ──────────────────────────────────────────
                        if status == "in_progress" {
                            let outer_content_type = content.get("content_type").and_then(|v| v.as_str()).unwrap_or("");
                            let mut new_text = String::new();

                            if outer_content_type == "text" {
                                let part = content
                                    .get("parts")
                                    .and_then(|v| v.as_array())
                                    .and_then(|a| a.first())
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");

                                if part.is_empty() {
                                    if role == "assistant" && self.last_role.as_deref() != Some("assistant") {
                                        new_text = if self.last_role.is_none() { String::new() } else { "\n".to_string() };
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
                                        let incoming = &part[slice_start..];
                                        if role == "assistant" && self.last_role.as_deref() != Some("assistant") {
                                            if recipient == "dalle.text2im" {
                                                new_text = format!("\n```{}\n{}", recipient, incoming);
                                            } else if recipient == "t2uay3k.sj1i4kz" {
                                                new_text = format!("\n```image_creator\n{}", incoming);
                                            } else if self.last_role.is_none() {
                                                new_text = incoming.to_string();
                                            } else {
                                                new_text = format!("\n\n{}", incoming);
                                            }
                                        } else if role == "tool" && self.last_role.as_deref() != Some("tool") {
                                            new_text = format!(">{}\n{}", initial_text, incoming);
                                        } else if role == "tool" {
                                            new_text = incoming.replace("\n\n", "\n");
                                        } else {
                                            new_text = incoming.to_string();
                                        }
                                    }
                                    self.len_last_content = part.len();
                                }
                            } else if outer_content_type == "multimodal_text" {
                                // ── in_progress multimodal_text（与 Python 对齐）──
                                let parts = content.get("parts").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                                for part in &parts {
                                    if let Some(asset_ptr) = part.get("asset_pointer").and_then(|v| v.as_str()) {
                                        let file_id = asset_ptr.replace("sediment://", "");
                                        let full_height = part.get("height").and_then(|v| v.as_u64()).unwrap_or(0);
                                        let current_height = part
                                            .get("metadata")
                                            .and_then(|m| m.get("generation"))
                                            .and_then(|g| g.get("height"))
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(0);
                                        if full_height > current_height {
                                            let rate = current_height as f64 / full_height as f64;
                                            new_text = format!("\n> {:.0}%\n", rate * 100.0);
                                            if self.last_role.as_deref() != Some(role) {
                                                new_text = format!("\n```{}", new_text);
                                            }
                                        } else {
                                            // 图像完成，获取下载链接
                                            let conv_id_str = conversation_id.as_str().unwrap_or("");
                                            let service = self.service.clone();
                                            let file_id_clone = file_id.clone();
                                            let conv_id_clone = conv_id_str.to_string();
                                            // 在同步 poll 里用 block_in_place 执行异步
                                            let image_url = tokio::task::block_in_place(|| {
                                                tokio::runtime::Handle::current().block_on(async {
                                                    service.get_attachment_url(&file_id_clone, &conv_id_clone).await
                                                })
                                            });
                                            if let Some(url) = image_url {
                                                new_text = format!("\n```\n![image]({})\n", url);
                                            }
                                        }
                                    }
                                }
                            } else {
                                let text = content.get("text").and_then(|v| v.as_str()).unwrap_or("");
                                let slice_start = self.len_last_content.min(text.len());
                                let incoming = &text[slice_start..];
                                if outer_content_type == "code" && self.last_content_type.as_deref() != Some("code") {
                                    let mut lang = content.get("language").and_then(|v| v.as_str()).unwrap_or("");
                                    if lang.is_empty() || lang == "unknown" { lang = recipient; }
                                    new_text = format!("\n```{}\n{}", lang, incoming);
                                } else if outer_content_type == "execution_output" && self.last_content_type.as_deref() != Some("execution_output") {
                                    new_text = format!("\n```Output\n{}", incoming);
                                } else {
                                    new_text = incoming.to_string();
                                }
                                self.len_last_content = text.len();
                            }

                            // 闭合 code/execution_output/multimodal_text 块
                            if self.last_content_type.as_deref() == Some("code") && outer_content_type != "code" {
                                new_text = format!("\n```\n{}", new_text);
                            } else if self.last_content_type.as_deref() == Some("execution_output") && outer_content_type != "execution_output" {
                                new_text = format!("\n```\n{}", new_text);
                            } else if self.last_content_type.as_deref() == Some("multimodal_text") && outer_content_type != "multimodal_text" {
                                new_text = format!("\n```\n{}", new_text);
                            }

                            self.last_content_type = Some(outer_content_type.to_string());

                            if self.completion_tokens >= self.max_tokens {
                                finish_reason = json!("length");
                                delta = json!({});
                                self.end = true;
                            } else {
                                finish_reason = Value::Null;
                                delta = json!({ "content": new_text });
                            }
                        }
                        // ── finished_successfully ─────────────────────────────────
                        else if status == "finished_successfully" {
                            let outer_content_type = content.get("content_type").and_then(|v| v.as_str()).unwrap_or("");
                            if outer_content_type == "multimodal_text" {
                                // 处理多模态完成（图片下载，对齐 Python）
                                let parts = content.get("parts").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                                let mut result_delta = json!({});
                                for part in &parts {
                                    if part.is_string() { continue; }
                                    let inner_ct = part.get("content_type").and_then(|v| v.as_str()).unwrap_or("");
                                    if inner_ct == "image_asset_pointer" {
                                        self.last_content_type = Some("image_asset_pointer".to_string());
                                        let asset_ptr = part.get("asset_pointer").and_then(|v| v.as_str()).unwrap_or("");
                                        if asset_ptr.starts_with("file-service://") {
                                            let fid = asset_ptr.replace("file-service://", "");
                                            debug!("file_id: {}", fid);
                                            let service = self.service.clone();
                                            let fid_clone = fid.clone();
                                            let image_url = tokio::task::block_in_place(|| {
                                                tokio::runtime::Handle::current().block_on(async {
                                                    service.get_download_url(&fid_clone).await
                                                })
                                            });
                                            debug!("image_download_url: {:?}", image_url);
                                            if let Some(url) = image_url {
                                                result_delta = json!({ "content": format!("\n```\n![image]({})\n", url) });
                                            } else {
                                                result_delta = json!({ "content": "\n```\nFailed to load the image.\n" });
                                            }
                                        } else {
                                            let fid = asset_ptr.replace("sediment://", "");
                                            let conv_id_str = conversation_id.as_str().unwrap_or("");
                                            let service = self.service.clone();
                                            let fid_clone = fid.clone();
                                            let conv_id_clone = conv_id_str.to_string();
                                            let image_url = tokio::task::block_in_place(|| {
                                                tokio::runtime::Handle::current().block_on(async {
                                                    service.get_attachment_url(&fid_clone, &conv_id_clone).await
                                                })
                                            });
                                            if let Some(url) = image_url {
                                                result_delta = json!({ "content": format!("\n![image]({})\n", url) });
                                            }
                                        }
                                    }
                                }
                                delta = result_delta;
                                finish_reason = Value::Null;
                                // multimodal finished_successfully 不一定是 end_turn，不在这里结束
                            } else if message.get("end_turn").and_then(|v| v.as_bool()).unwrap_or(false) {
                                // 普通文本 end_turn
                                let part_str = content
                                    .get("parts")
                                    .and_then(|v| v.as_array())
                                    .and_then(|a| a.first())
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let new_text = &part_str[self.len_last_content.min(part_str.len())..];
                                if !new_text.is_empty() {
                                    delta = json!({ "content": new_text });
                                } else {
                                    // 检查 sandbox 文件链接（对齐 Python regex 逻辑）
                                    let sandbox_re = Regex::new(r"\(sandbox:(.*?)\)").unwrap();
                                    let matches: Vec<&str> = sandbox_re
                                        .captures_iter(part_str)
                                        .filter_map(|c| c.get(1).map(|m| m.as_str()))
                                        .collect();
                                    if !matches.is_empty() {
                                        let service = self.service.clone();
                                        let conv_id_str = conversation_id.as_str().unwrap_or("").to_string();
                                        let msg_id_str = message_id.clone().unwrap_or_default();
                                        let sandbox_paths: Vec<String> = matches.iter().map(|s| s.to_string()).collect();
                                        let file_url_content = tokio::task::block_in_place(|| {
                                            tokio::runtime::Handle::current().block_on(async {
                                                let mut content_str = String::new();
                                                for (i, sp) in sandbox_paths.iter().enumerate() {
                                                    if let Some(url) = service.get_response_file_url(&conv_id_str, &msg_id_str, sp).await {
                                                        content_str.push_str(&format!("\n```\n\n![File {}]({})\n", i + 1, url));
                                                    }
                                                }
                                                content_str
                                            })
                                        });
                                        delta = json!({ "content": file_url_content });
                                    } else {
                                        delta = json!({});
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
                                finish_reason = Value::Null;
                            }
                        } else {
                            continue;
                        }

                        self.last_message_id = message_id.clone();
                        self.last_role = Some(role.to_string());
                        self.last_status = Some(status.to_string());

                        // 与 Python 对齐：没有 content 字段时输出空 delta
                        let effective_delta = if !self.end {
                            let has_no_content = delta.get("content").is_none();
                            let is_empty_content = delta.get("content")
                                .and_then(|v| v.as_str())
                                .map_or(false, |s| s.is_empty());
                            if has_no_content || is_empty_content {
                                json!({ "role": "assistant", "content": "" })
                            } else {
                                delta
                            }
                        } else {
                            delta
                        };

                        let chunk = build_chunk(
                            &self.chat_id,
                            self.created_time,
                            &self.model,
                            &self.system_fingerprint,
                            effective_delta,
                            finish_reason,
                            &message_id,
                            &conversation_id,
                            self.service.history_disabled,
                        );
                        self.completion_tokens += 1;
                        output_bytes.extend_from_slice(format!("data: {}\n\n", chunk).as_bytes());
                    }

                    if !output_bytes.is_empty() {
                        return Poll::Ready(Some(Ok(Bytes::from(output_bytes))));
                    }
                }
            }
        }
    }
}

/// 构建一个 SSE chunk JSON（抽出为辅助函数，减少重复）
fn build_chunk(
    chat_id: &str,
    created_time: i64,
    model: &str,
    system_fingerprint: &Option<String>,
    delta: Value,
    finish_reason: Value,
    message_id: &Option<String>,
    conversation_id: &Value,
    history_disabled: bool,
) -> Value {
    let mut chunk = json!({
        "id": chat_id,
        "object": "chat.completion.chunk",
        "created": created_time,
        "model": model,
        "choices": [{
            "index": 0,
            "delta": delta,
            "logprobs": null,
            "finish_reason": finish_reason
        }]
    });
    if let Some(fp) = system_fingerprint {
        chunk.as_object_mut().unwrap().insert("system_fingerprint".to_string(), json!(fp));
    }
    if !history_disabled {
        if let Some(mid) = message_id {
            chunk.as_object_mut().unwrap().insert("message_id".to_string(), json!(mid));
        }
        chunk.as_object_mut().unwrap().insert("conversation_id".to_string(), conversation_id.clone());
    }
    chunk
}

/// 非流式响应转换（与 Python format_not_stream_response 对齐）
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
            // 与 Python 对齐：用 try/except 包裹，出错直接 continue
            let data_json: Value = match serde_json::from_str(&line[6..]) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if let Some(choices) = data_json.get("choices").and_then(|v| v.as_array()) {
                if let Some(first_choice) = choices.first() {
                    // 与 Python 对齐：检查 delta 是否存在
                    let delta = match first_choice.get("delta") {
                        Some(d) if !d.is_null() => d,
                        _ => continue,  // Skip if delta is missing or null
                    };
                    // 与 Python 对齐：delta 里没有 content key 时 skip（不崩溃）
                    if let Some(content_val) = delta.get("content") {
                        if let Some(content_str) = content_val.as_str() {
                            all_text.push_str(content_str);
                        }
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
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content
            },
            "logprobs": null,
            "finish_reason": finish_reason
        }],
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
