use actix_web::{get, post, web, HttpRequest, HttpResponse, Responder};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use tera::{Context, Tera};
use log::{info, error};

use crate::config::Config;
use crate::globals::AppState;
use crate::chatgpt::service::ChatService;
use crate::chatgpt::format::{OpenAIStream, format_not_stream_response, api_messages_to_chat};

#[derive(Deserialize)]
pub struct TokenUploadForm {
    pub text: String,
}

// 核心接口：/v1/chat/completions
#[post("/v1/chat/completions")]
pub async fn send_conversation(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
    state: web::Data<AppState>,
    config: web::Data<Config>,
) -> impl Responder {
    // 提取 Authorization Bearer Token
    let auth_header = req.headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    
    let origin_token = if auth_header.starts_with("Bearer ") {
        Some(auth_header[7..].to_string())
    } else if !auth_header.is_empty() {
        Some(auth_header.to_string())
    } else {
        None
    };

    let req_body = body.into_inner();
    let max_retries = config.retry_times;
    let mut last_err = None;

    let is_stream = req_body.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    let max_tokens = req_body.get("max_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(usize::MAX as u64) as usize;
    let api_messages = req_body.get("messages").cloned().unwrap_or(json!([]));
    let parent_msg_id = req_body.get("parent_message_id").and_then(|v| v.as_str());

    for attempt in 0..=max_retries {
        if attempt > 0 {
            info!("Retrying send_conversation (attempt {}/{})...", attempt, max_retries);
        }

        // 1. 初始化 ChatService 实例
        let service = match ChatService::new(
            state.get_ref().clone(),
            config.get_ref().clone(),
            origin_token.clone(),
            &req_body,
        ).await {
            Ok(s) => s,
            Err(e) => {
                error!("Attempt {} failed: ChatService::new error: {:?}", attempt, e);
                last_err = Some(e);
                continue;
            }
        };

        let mut service = service;

        // 2. Sentinel 握手与解决 POW
        if let Err(e) = service.get_chat_requirements().await {
            error!("Attempt {} failed: Sentinel handshake error: {:?}", attempt, e);
            last_err = Some(e);
            continue;
        }

        let upload_by_url = service.config.upload_by_url;

        // 3. 格式化 OpenAI messages 列表到 ChatGPT 消息协议格式
        let (chat_messages, prompt_tokens) = match api_messages_to_chat(&service, &api_messages, upload_by_url).await {
            Ok(res) => res,
            Err(e) => {
                error!("Attempt {} failed: api_messages_to_chat error: {:?}", attempt, e);
                last_err = Some(e);
                continue;
            }
        };

        // 4. 准备请求体并发送会话请求
        let chat_req_body = service.prepare_send_conversation(chat_messages, parent_msg_id).await;
        let response = match service.send_conversation_request(chat_req_body).await {
            Ok(resp) => resp,
            Err(e) => {
                error!("Attempt {} failed: send_conversation_request error: {:?}", attempt, e);
                last_err = Some(e);
                continue;
            }
        };

        // 成功建立连接，退出重试
        let service_arc = Arc::new(service);
        let model = service_arc.resp_model.clone();
        
        // 把原始流的 bytes 传递下去
        let raw_stream = Box::pin(response.bytes_stream());
        let openai_stream = OpenAIStream::new(service_arc.clone(), raw_stream, model.clone(), max_tokens);

        if is_stream {
            return HttpResponse::Ok()
                .content_type("text/event-stream")
                .streaming(openai_stream);
        } else {
            // 非流式：将流式数据融合成一个最终 JSON 结构
            match format_not_stream_response(openai_stream, prompt_tokens, max_tokens, model).await {
                Ok(json_res) => return HttpResponse::Ok().json(json_res),
                Err(e) => {
                    error!("Attempt {} failed: format_not_stream_response error: {:?}", attempt, e);
                    last_err = Some(e);
                    continue;
                }
            }
        }
    }

    // 所有重试都失败了
    let final_err = last_err.unwrap_or_else(|| actix_web::error::ErrorInternalServerError("Unknown server error"));
    HttpResponse::from_error(final_err)
}

// tokens.html 渲染页面
#[get("/tokens")]
pub async fn upload_html(
    state: web::Data<AppState>,
    config: web::Data<Config>,
    tera: web::Data<Tera>,
) -> impl Responder {
    let inner = state.inner.read().await;
    let mut tokens_count = 0;
    for t in &inner.token_list {
        if !inner.error_token_list.contains(t) {
            tokens_count += 1;
        }
    }

    let mut context = Context::new();
    context.insert("api_prefix", &config.api_prefix.clone().unwrap_or_default());
    context.insert("tokens_count", &tokens_count);

    match tera.render("tokens.html", &context) {
        Ok(html) => HttpResponse::Ok().content_type("text/html").body(html),
        Err(e) => {
            error!("Tera render error: {:?}", e);
            HttpResponse::InternalServerError().body("Internal Server Error")
        }
    }
}

// 批量上传 Token：/tokens/upload
#[post("/tokens/upload")]
pub async fn upload_post(
    form: web::Form<TokenUploadForm>,
    state: web::Data<AppState>,
) -> impl Responder {
    let mut added_count = 0;
    let lines = form.text.split('\n');
    
    for line in lines {
        let trimmed = line.trim();
        if !trimmed.is_empty() && !trimmed.starts_with('#') {
            state.append_token(trimmed).await;
            added_count += 1;
        }
    }

    let inner = state.inner.read().await;
    let active_count = inner.token_list.iter()
        .filter(|t| !inner.error_token_list.contains(t))
        .count();

    info!("Uploaded {} tokens. Current active tokens: {}", added_count, active_count);

    HttpResponse::Ok().json(json!({
        "status": "success",
        "tokens_count": active_count
    }))
}

// 清空 Token 列表：/tokens/clear
#[post("/tokens/clear")]
pub async fn clear_tokens(
    state: web::Data<AppState>,
) -> impl Responder {
    state.clear_tokens().await;
    info!("Tokens cleared.");
    
    HttpResponse::Ok().json(json!({
        "status": "success",
        "tokens_count": 0
    }))
}

// 获取错误 Token 列表：/tokens/error
#[post("/tokens/error")]
pub async fn error_tokens(
    state: web::Data<AppState>,
) -> impl Responder {
    let inner = state.inner.read().await;
    HttpResponse::Ok().json(json!({
        "status": "success",
        "error_tokens": inner.error_token_list
    }))
}

// 追加单个 Token：/tokens/add/{token}
#[get("/tokens/add/{token}")]
pub async fn add_token(
    path: web::Path<String>,
    state: web::Data<AppState>,
) -> impl Responder {
    let token = path.into_inner();
    let trimmed = token.trim();
    if !trimmed.is_empty() && !trimmed.starts_with('#') {
        state.append_token(trimmed).await;
    }

    let inner = state.inner.read().await;
    let active_count = inner.token_list.iter()
        .filter(|t| !inner.error_token_list.contains(t))
        .count();

    HttpResponse::Ok().json(json!({
        "status": "success",
        "tokens_count": active_count
    }))
}
