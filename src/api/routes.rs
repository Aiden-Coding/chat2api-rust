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

/// 用于批量上传 Token 的表单数据结构
#[derive(Deserialize)]
pub struct TokenUploadForm {
    pub text: String, // 包含一行一个 Token 的多行文本
}

/// 核心接口：/v1/chat/completions
/// 兼容 OpenAI 协议的会话接口，支持流式与非流式问答，并提供最大重试轮询保护
#[post("/v1/chat/completions")]
pub async fn send_conversation(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
    state: web::Data<AppState>,
    config: web::Data<Config>,
) -> impl Responder {
    // 1. 从请求头中提取 Authorization Bearer Token
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
    let max_retries = config.retry_times; // 最大重试次数
    let mut last_err = None;

    // 解析请求参数
    let is_stream = req_body.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    let max_tokens = req_body.get("max_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(usize::MAX as u64) as usize;
    let api_messages = req_body.get("messages").cloned().unwrap_or(json!([]));
    let parent_msg_id = req_body.get("parent_message_id").and_then(|v| v.as_str());

    // 开启错误重试循环
    for attempt in 0..=max_retries {
        if attempt > 0 {
            info!("正在重试发送会话 (第 {}/{} 次重试)...", attempt, max_retries);
        }

        // 步骤 2.1: 初始化 ChatService 实例（其中会在此处拦截本地限流频控）
        let service = match ChatService::new(
            state.get_ref().clone(),
            config.get_ref().clone(),
            origin_token.clone(),
            &req_body,
        ).await {
            Ok(s) => s,
            Err(e) => {
                error!("第 {} 次尝试失败: 初始化 ChatService 错误: {:?}", attempt, e);
                last_err = Some(e);
                continue; // 错误后轮询下一个账号重试
            }
        };

        let mut service = service;

        // 步骤 2.2: 进行 Sentinel 握手解密并解决 POW 工作量证明，必要时向远程求解 Arkose
        if let Err(e) = service.get_chat_requirements().await {
            error!("第 {} 次尝试失败: Sentinel 握手握手失败: {:?}", attempt, e);
            last_err = Some(e);
            continue;
        }

        let upload_by_url = service.config.upload_by_url;

        // 步骤 2.3: 格式化 OpenAI 消息协议，如果是多模态请求则在此处进行图片代理下载并获取宽高
        let (chat_messages, prompt_tokens) = match api_messages_to_chat(&service, &api_messages, upload_by_url).await {
            Ok(res) => res,
            Err(e) => {
                error!("第 {} 次尝试失败: 消息协议转换错误: {:?}", attempt, e);
                last_err = Some(e);
                continue;
            }
        };

        // 步骤 2.4: 组装 ChatGPT 官方的请求体格式并发送 POST 对话请求
        let chat_req_body = service.prepare_send_conversation(chat_messages, parent_msg_id).await;
        let response = match service.send_conversation_request(chat_req_body).await {
            Ok(resp) => resp,
            Err(e) => {
                error!("第 {} 次尝试失败: 发送对话请求异常: {:?}", attempt, e);
                last_err = Some(e);
                continue;
            }
        };

        // 成功与 OpenAI 握手建立连接，退出重试逻辑
        let service_arc = Arc::new(service);
        let model = service_arc.resp_model.clone();
        
        // 获得底层的 HTTP 响应流，并构建包装成 OpenAIStream 结构体
        let raw_stream = Box::pin(response.bytes_stream());
        let openai_stream = OpenAIStream::new(service_arc.clone(), raw_stream, model.clone(), max_tokens);

        // 如果是流式推送 SSE (Event-Stream)
        if is_stream {
            return HttpResponse::Ok()
                .content_type("text/event-stream")
                .streaming(openai_stream);
        } else {
            // 非流式：在内存中汇聚并整合流式响应块，最终输出一个完整的答复 JSON
            match format_not_stream_response(openai_stream, prompt_tokens, max_tokens, model).await {
                Ok(json_res) => return HttpResponse::Ok().json(json_res),
                Err(e) => {
                    error!("第 {} 次尝试失败: 非流式聚合发生错误: {:?}", attempt, e);
                    last_err = Some(e);
                    continue;
                }
            }
        }
    }

    // 所有可用 Token 及重试全数失败，返回最后一次抛出的错误
    let final_err = last_err.unwrap_or_else(|| actix_web::error::ErrorInternalServerError("Unknown server error"));
    HttpResponse::from_error(final_err)
}

/// 渲染批量 tokens 管理的前端 html 页面：GET /tokens
#[get("/tokens")]
pub async fn upload_html(
    state: web::Data<AppState>,
    config: web::Data<Config>,
    tera: web::Data<Tera>,
) -> impl Responder {
    let inner = state.inner.read().await;
    let mut tokens_count = 0;
    // 统计目前活跃的健康 Token 数量（排除 error_token_list 中的坏 Token）
    for t in &inner.token_list {
        if !inner.error_token_list.contains(t) {
            tokens_count += 1;
        }
    }

    let mut context = Context::new();
    context.insert("api_prefix", &config.api_prefix.clone().unwrap_or_default());
    context.insert("tokens_count", &tokens_count);

    // 采用 Tera 渲染引擎渲染 tokens.html 模板
    match tera.render("tokens.html", &context) {
        Ok(html) => HttpResponse::Ok().content_type("text/html").body(html),
        Err(e) => {
            error!("Tera 模板渲染异常: {:?}", e);
            HttpResponse::InternalServerError().body("Internal Server Error")
        }
    }
}

/// 批量上传/导入 Token：POST /tokens/upload (由 tokens.html 表单触发)
#[post("/tokens/upload")]
pub async fn upload_post(
    form: web::Form<TokenUploadForm>,
    state: web::Data<AppState>,
) -> impl Responder {
    let mut added_count = 0;
    let lines = form.text.split('\n');
    
    // 解析每一行 Token 并存入内存列表
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

    info!("成功上传了 {} 个 Tokens。当前处于活跃状态的 Tokens: {}", added_count, active_count);

    HttpResponse::Ok().json(json!({
        "status": "success",
        "tokens_count": active_count
    }))
}

/// 清空所有的 Token 列表：POST /tokens/clear
#[post("/tokens/clear")]
pub async fn clear_tokens(
    state: web::Data<AppState>,
) -> impl Responder {
    state.clear_tokens().await;
    info!("已成功清空所有的 Token。");
    
    HttpResponse::Ok().json(json!({
        "status": "success",
        "tokens_count": 0
    }))
}

/// 获取当前所有标记为异常的错误 Token 列表：POST /tokens/error
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

/// 获取当前所有 Token 列表：GET /tokens/list
#[get("/tokens/list")]
pub async fn get_token_list(
    state: web::Data<AppState>,
) -> impl Responder {
    let inner = state.inner.read().await;
    HttpResponse::Ok().json(json!({
        "status": "success",
        "tokens": inner.token_list,
        "error_tokens": inner.error_token_list
    }))
}

/// 快速追加单个 Token 到内存和文件：GET /tokens/add/{token}
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

/// 清空全局的 Seed 种子映射和会话隔离关系表：POST /seed_tokens/clear
#[post("/seed_tokens/clear")]
pub async fn clear_seed_tokens(
    state: web::Data<AppState>,
) -> impl Responder {
    state.clear_seed_tokens().await;
    info!("已成功清空全局的 Seed Tokens 映射关系。");
    
    HttpResponse::Ok().json(json!({
        "status": "success",
        "seed_tokens_count": 0
    }))
}
