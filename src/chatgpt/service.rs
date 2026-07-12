use std::collections::HashMap;
use std::sync::Arc;
use rand::seq::SliceRandom;
use serde_json::{json, Value};
use uuid::Uuid;
use log::{info, error, debug};
use actix_web::error::{ErrorInternalServerError, ErrorForbidden, ErrorNotFound, ErrorBadRequest};
use rquest::header::{HeaderMap, HeaderName, HeaderValue};
use md5;

use crate::config::Config;
use crate::globals::AppState;
use crate::chatgpt::client::create_client;
use crate::chatgpt::auth::{verify_token, get_req_token};
use crate::chatgpt::pow::{get_config, get_requirements_token, get_answer_token};
use crate::chatgpt::turnstile::process_turnstile;

pub struct FileMeta {
    pub file_id: String,
    pub size_bytes: usize,
    pub file_name: String,
    pub mime_type: String,
    pub use_case: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

pub struct ChatService {
    pub state: AppState,
    pub config: Config,
    pub origin_token: Option<String>,
    pub req_token: String,
    pub access_token: Option<String>,
    pub account_id: Option<String>,
    
    pub client: rquest::Client,
    pub sentinel_client: rquest::Client,
    
    pub host_url: String,
    pub base_url: String,
    pub base_headers: HeaderMap,
    pub user_agent: String,
    pub impersonate: String,
    
    pub origin_model: String,
    pub resp_model: String,
    pub req_model: String,
    pub gizmo_id: Option<String>,

    pub chat_token: String,
    pub proof_token: Option<String>,
    pub ark0se_token: Option<String>,
    pub turnstile_token: Option<String>,
    
    pub history_disabled: bool,
    pub check_model: bool,
}

use std::sync::Mutex;

// 静态 DPL 缓存
static CACHED_DPL: Mutex<Option<String>> = Mutex::new(None);
static CACHED_SCRIPT: Mutex<Option<String>> = Mutex::new(None);
static CACHED_TIME: Mutex<i64> = Mutex::new(0);

impl ChatService {
    pub async fn new(
        state: AppState,
        config: Config,
        origin_token: Option<String>,
        data: &Value,
    ) -> Result<Self, actix_web::Error> {
        let req_token = get_req_token(&state, &config, origin_token.as_deref().unwrap_or(""), data.get("seed").and_then(|v| v.as_str())).await?;
        
        let mut access_token = None;
        let mut account_id = None;

        if !req_token.is_empty() {
            let split_tok: Vec<&str> = req_token.split(',').collect();
            if split_tok.len() == 1 {
                access_token = verify_token(&state, &config, split_tok[0]).await?;
            } else if split_tok.len() >= 2 {
                access_token = verify_token(&state, &config, split_tok[0]).await?;
                account_id = Some(split_tok[1].to_string());
            }
        }

        // 使用默认或从 globals 取 fp (本例简写随机指纹)
        let impersonate = "safari15_3".to_string();
        let user_agent = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/15.3 Safari/605.1.15".to_string();

        let host_url = if !config.chatgpt_base_url_list.is_empty() {
            let mut rng = rand::thread_rng();
            config.chatgpt_base_url_list.choose(&mut rng).unwrap().clone()
        } else {
            "https://chatgpt.com".to_string()
        };

        let digest = md5::compute(req_token.as_bytes());
        let session_id = format!("{:x}", digest);
        
        let main_proxy = if !config.proxy_url_list.is_empty() {
            let mut rng = rand::thread_rng();
            Some(config.proxy_url_list.choose(&mut rng).unwrap().replace("{}", &session_id))
        } else {
            None
        };

        let sentinel_proxy = if !config.sentinel_proxy_url_list.is_empty() {
            let mut rng = rand::thread_rng();
            Some(config.sentinel_proxy_url_list.choose(&mut rng).unwrap().replace("{}", &session_id))
        } else {
            main_proxy.clone()
        };

        let client = create_client(main_proxy.as_deref())
            .map_err(|e| ErrorInternalServerError(format!("Failed to create client: {:?}", e)))?;
        let sentinel_client = create_client(sentinel_proxy.as_deref())
            .map_err(|e| ErrorInternalServerError(format!("Failed to create sentinel client: {:?}", e)))?;

        let mut base_headers = HeaderMap::new();
        base_headers.insert("accept", HeaderValue::from_static("*/*"));
        base_headers.insert("accept-encoding", HeaderValue::from_static("gzip, deflate, br, zstd"));
        base_headers.insert("accept-language", HeaderValue::from_static("en-US,en;q=0.9"));
        base_headers.insert("content-type", HeaderValue::from_static("application/json"));
        base_headers.insert("oai-device-id", HeaderValue::from_str(&Uuid::new_v4().to_string()).unwrap());
        base_headers.insert("oai-language", HeaderValue::from_str(&config.oai_language).unwrap());
        base_headers.insert("origin", HeaderValue::from_str(&host_url).unwrap());
        base_headers.insert("priority", HeaderValue::from_static("u=1, i"));
        base_headers.insert("referer", HeaderValue::from_str(&format!("{}/", host_url)).unwrap());
        base_headers.insert("sec-fetch-dest", HeaderValue::from_static("empty"));
        base_headers.insert("sec-fetch-mode", HeaderValue::from_static("cors"));
        base_headers.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
        base_headers.insert("user-agent", HeaderValue::from_str(&user_agent).unwrap());

        let base_url = if access_token.is_some() {
            base_headers.insert("authorization", HeaderValue::from_str(&format!("Bearer {}", access_token.as_ref().unwrap())).unwrap());
            if let Some(ref acc_id) = account_id {
                base_headers.insert("chatgpt-account-id", HeaderValue::from_str(acc_id).unwrap());
            }
            format!("{}/backend-api", host_url)
        } else {
            format!("{}/backend-anon", host_url)
        };

        if let Some(ref a_key) = config.auth_key {
            base_headers.insert("authkey", HeaderValue::from_str(a_key).unwrap());
        }

        // 处理模型名字解析
        let origin_model = data.get("model").and_then(|v| v.as_str()).unwrap_or("gpt-3.5-turbo").to_string();
        let mut model_map = HashMap::new();
        model_map.insert("gpt-3.5-turbo", "gpt-3.5-turbo-0125");
        model_map.insert("gpt-4", "gpt-4-0613");
        model_map.insert("gpt-4o", "gpt-4o-2024-08-06");
        model_map.insert("gpt-4o-mini", "gpt-4o-mini-2024-07-18");
        model_map.insert("o1-preview", "o1-preview-2024-09-12");
        model_map.insert("o1-mini", "o1-mini-2024-09-12");
        model_map.insert("o1", "o1-2024-12-18");
        model_map.insert("o3-mini", "o3-mini-2025-01-31");
        
        let resp_model = model_map.get(origin_model.as_str()).cloned().unwrap_or(origin_model.as_str()).to_string();
        let gizmo_id = if origin_model.contains("gizmo") || origin_model.contains("g-") {
            origin_model.split("g-").last().map(|s| format!("g-{}", s))
        } else {
            None
        };

        let req_model = if origin_model.contains("o3-mini-high") {
            "o3-mini-high"
        } else if origin_model.contains("o3-mini") {
            "o3-mini"
        } else if origin_model.contains("o1-preview") {
            "o1-preview"
        } else if origin_model.contains("o1-mini") {
            "o1-mini"
        } else if origin_model.contains("gpt-4o-mini") {
            "gpt-4o-mini"
        } else if origin_model.contains("gpt-4o") {
            "gpt-4o"
        } else if origin_model.contains("gpt-4") {
            "gpt-4"
        } else {
            "text-davinci-002-render-sha" // 3.5 降级
        }.to_string();

        let history_disabled = data.get("history_disabled").and_then(|v| v.as_bool()).unwrap_or(config.history_disabled);

        let mut service = Self {
            state,
            config,
            origin_token,
            req_token,
            access_token,
            account_id,
            client,
            sentinel_client,
            host_url,
            base_url,
            base_headers,
            user_agent,
            impersonate,
            origin_model,
            resp_model,
            req_model,
            gizmo_id,
            chat_token: "gAAAAAB".to_string(),
            proof_token: None,
            ark0se_token: None,
            turnstile_token: None,
            history_disabled,
            check_model: false,
        };

        service.get_dpl().await?;

        Ok(service)
    }

    // 更新 DPL
    pub async fn get_dpl(&mut self) -> Result<(), actix_web::Error> {
        let now = chrono::Utc::now().timestamp();
        {
            let cached_time = CACHED_TIME.lock().unwrap();
            let cached_dpl = CACHED_DPL.lock().unwrap();
            if now - *cached_time < 15 * 60 && cached_dpl.is_some() {
                return Ok(());
            }
        }
        
        if self.config.conversation_only {
            return Ok(());
        }

        let resp_res = self.client.get(&self.host_url)
            .headers(self.base_headers.clone())
            .send()
            .await;

        if let Ok(resp) = resp_res {
            if resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                // 正则提取 data-build 或者是 script
                if let Some(caps) = regex::Regex::new(r#"data-build="([^"]*)""#).unwrap().captures(&body) {
                    let dpl = caps.get(1).map(|m: regex::Match| m.as_str().to_string()).unwrap_or_default();
                    *CACHED_DPL.lock().unwrap() = Some(dpl.clone());
                    *CACHED_SCRIPT.lock().unwrap() = Some("https://chatgpt.com/backend-api/sentinel/sdk.js".to_string());
                    *CACHED_TIME.lock().unwrap() = now;
                    info!("Found DPL: {:?}", dpl);
                }
            }
        }

        {
            let mut cached_dpl = CACHED_DPL.lock().unwrap();
            if cached_dpl.is_none() {
                *cached_dpl = Some("prod-f501fe933b3edf57aea882da888e1a544df99840".to_string());
                *CACHED_SCRIPT.lock().unwrap() = Some("https://chatgpt.com/backend-api/sentinel/sdk.js".to_string());
            }
        }
        Ok(())
    }

    // 获取 Sentinel chat-requirements 与解决 POW
    pub async fn get_chat_requirements(&mut self) -> Result<(), actix_web::Error> {
        if self.config.conversation_only {
            return Ok(());
        }

        let url = format!("{}/sentinel/chat-requirements", self.base_url);
        let cached_dpl = CACHED_DPL.lock().unwrap().clone().unwrap_or_default();
        let cached_script = CACHED_SCRIPT.lock().unwrap().clone().unwrap_or_default();

        let config_val = get_config(&self.user_agent, &cached_dpl, &cached_script);
        let p_token = get_requirements_token(&config_val);

        let payload = json!({ "p": p_token });

        let resp_res = self.sentinel_client.post(&url)
            .headers(self.base_headers.clone())
            .json(&payload)
            .send()
            .await;

        match resp_res {
            Ok(resp) => {
                let status = resp.status();
                info!("Sentinel response status: {}", status);
                if status.is_success() {
                    let text = resp.text().await.unwrap_or_default();
                    info!("Sentinel response body: {}", text);
                    let json_val: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                    
                    self.chat_token = json_val.get("token").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    if self.chat_token.is_empty() {
                        return Err(ErrorForbidden(format!("Failed to get chat requirements token, body was: {}", serde_json::to_string(&json_val).unwrap_or_default())));
                    }

                    // 校验模型权限
                    let persona = json_val.get("persona").and_then(|v| v.as_str()).unwrap_or("");
                    if persona != "chatgpt-paid" && (self.req_model == "gpt-4" || self.req_model == "o1-preview") {
                        return Err(ErrorNotFound(json!({
                            "message": format!("The model `{}` does not exist or you do not have access to it.", self.origin_model),
                            "type": "invalid_request_error",
                            "code": "model_not_found"
                        }).to_string()));
                    }

                    // 处理 Turnstile 如果 required
                    if let Some(turnstile) = json_val.get("turnstile") {
                        if turnstile.get("required").and_then(|v| v.as_bool()).unwrap_or(false) {
                            if let Some(dx) = turnstile.get("dx").and_then(|v| v.as_str()) {
                                self.turnstile_token = Some(process_turnstile(dx, &p_token));
                            }
                        }
                    }

                    // 处理 Proof of Work 如果 required
                    if let Some(pow) = json_val.get("proofofwork") {
                        if pow.get("required").and_then(|v| v.as_bool()).unwrap_or(false) {
                            let diff = pow.get("difficulty").and_then(|v| v.as_str()).unwrap_or("000032");
                            let seed = pow.get("seed").and_then(|v| v.as_str()).unwrap_or("");
                            
                            // 调用多线程求解器
                            let (ans, solved) = get_answer_token(seed, diff, &config_val);
                            if solved {
                                self.proof_token = Some(ans);
                            } else {
                                return Err(ErrorForbidden("Failed to solve proof of work"));
                            }
                        }
                    }
                    Ok(())
                } else {
                    let err_text = resp.text().await.unwrap_or_default();
                    error!("Sentinel non-200 status {}, body: {}", status, err_text);
                    Err(ErrorForbidden(format!("Sentinel returns status {}, detail: {}", status, err_text)))
                }
            }
            Err(e) => {
                error!("Sentinel request error: {:?}", e);
                Err(ErrorInternalServerError(format!("Sentinel handshake failed: {:?}", e)))
            }
        }
    }

    // 拼装请求 Body
    pub async fn prepare_send_conversation(&self, chat_messages: Value, parent_message_id: Option<&str>) -> Value {
        let conversation_mode = if let Some(ref gid) = self.gizmo_id {
            json!({ "kind": "gizmo_interaction", "gizmo_id": gid })
        } else {
            json!({ "kind": "primary_assistant" })
        };

        let mut req_body = json!({
            "action": "next",
            "client_contextual_info": {
                "is_dark_mode": false,
                "time_since_loaded": 150,
                "page_height": 900,
                "page_width": 1400,
                "pixel_ratio": 1.5,
                "screen_height": 1080,
                "screen_width": 1920
            },
            "conversation_mode": conversation_mode,
            "conversation_origin": null,
            "force_paragen": false,
            "force_paragen_model_slug": "",
            "force_rate_limit": false,
            "force_use_sse": true,
            "history_and_training_disabled": self.history_disabled,
            "messages": chat_messages,
            "model": self.req_model,
            "paragen_cot_summary_display_override": "allow",
            "paragen_stream_type_override": null,
            "parent_message_id": parent_message_id.unwrap_or(&Uuid::new_v4().to_string()).to_string(),
            "reset_rate_limits": false,
            "suggestions": [],
            "supported_encodings": [],
            "system_hints": [],
            "timezone": "America/Los_Angeles",
            "timezone_offset_min": -480,
            "variant_purpose": "comparison_implicit",
            "websocket_request_id": Uuid::new_v4().to_string()
        });

        req_body
    }

    // 发送会话请求
    pub async fn send_conversation_request(&self, req_body: Value) -> Result<rquest::Response, actix_web::Error> {
        let url = format!("{}/conversation", self.base_url);
        let mut headers = self.base_headers.clone();
        headers.insert("accept", HeaderValue::from_static("text/event-stream"));
        headers.insert("accept-encoding", HeaderValue::from_static("identity"));
        
        if !self.config.conversation_only {
            headers.insert("openai-sentinel-chat-requirements-token", HeaderValue::from_str(&self.chat_token).unwrap());
            if let Some(ref proof) = self.proof_token {
                headers.insert("openai-sentinel-proof-token", HeaderValue::from_str(proof).unwrap());
            }
            if let Some(ref turnstile) = self.turnstile_token {
                headers.insert("openai-sentinel-turnstile-token", HeaderValue::from_str(turnstile).unwrap());
            }
        }

        let resp_res = self.client.post(&url)
            .headers(headers)
            .json(&req_body)
            .send()
            .await;

        match resp_res {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    Ok(resp)
                } else {
                    let err_text = resp.text().await.unwrap_or_default();
                    Err(ErrorInternalServerError(format!("OpenAI conversation failed, status {}, detail: {}", status, err_text)))
                }
            }
            Err(e) => Err(ErrorInternalServerError(format!("Request to OpenAI conversation failed: {:?}", e)))
        }
    }

    // 辅助多模态文件下载
    pub async fn get_file_content_from_url(&self, url: &str) -> Result<(Vec<u8>, String), actix_web::Error> {
        let resp = self.client.get(url)
            .send()
            .await
            .map_err(|e| ErrorBadRequest(format!("Failed to fetch file URL: {:?}", e)))?;
        let mime = resp.headers().get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let bytes = resp.bytes().await
            .map_err(|e| ErrorBadRequest(format!("Failed to read file bytes: {:?}", e)))?;
        Ok((bytes.to_vec(), mime))
    }

    // 文件上传占位接口（用于支持完整逆向的多模态能力）
    pub async fn upload_file(&self, _content: &[u8], _mime_type: &str) -> Result<Option<FileMeta>, actix_web::Error> {
        // 生产级逻辑通常是先请求 /backend-api/files 分配 file_id，
        // 然后 PUT 对应预签名 URL，最后请求 /backend-api/files/{file_id}/uploaded
        // 咱们可以先返回空或者实现简单占位
        Ok(None)
    }

    pub async fn check_upload(&self, _file_id: &str) -> bool {
        true
    }
}
