use std::collections::HashMap;
use rand::seq::SliceRandom;
use rand::Rng;
use serde_json::{json, Value};
use uuid::Uuid;
use log::{info, error};
use actix_web::error::{ErrorInternalServerError, ErrorForbidden, ErrorNotFound, ErrorBadRequest};
use rquest::header::{HeaderMap, HeaderValue};
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

    // 从请求体传入
    pub conversation_id: Option<String>,
    pub parent_message_id: Option<String>,
    pub max_tokens: usize,
}

use std::sync::Mutex;

// 静态 DPL 缓存
static CACHED_DPL: Mutex<Option<String>> = Mutex::new(None);
static CACHED_SCRIPT: Mutex<Option<String>> = Mutex::new(None);
static CACHED_TIME: Mutex<i64> = Mutex::new(0);

/// 根据 mime_type 确定文件扩展名
fn get_file_extension(mime_type: &str) -> &'static str {
    match mime_type {
        "image/jpeg" | "image/jpg" => ".jpg",
        "image/png" => ".png",
        "image/gif" => ".gif",
        "image/webp" => ".webp",
        "application/pdf" => ".pdf",
        "text/plain" => ".txt",
        "text/csv" => ".csv",
        "application/json" => ".json",
        "application/msword" => ".doc",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => ".docx",
        _ => ".bin",
    }
}

/// 根据 mime_type 确定 use_case
fn determine_file_use_case(mime_type: &str) -> &'static str {
    if mime_type.starts_with("image/") {
        "multimodal"
    } else {
        "ace_upload"
    }
}

/// 从图片字节中粗略读取宽高（支持 JPEG/PNG/GIF/WEBP）
fn get_image_size(data: &[u8]) -> Option<(u32, u32)> {
    // PNG: signature 8 bytes, then IHDR chunk: 4(len) + 4(type) + 4(w) + 4(h)
    if data.len() >= 24 && &data[0..8] == b"\x89PNG\r\n\x1a\n" {
        let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
        let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
        return Some((w, h));
    }
    // JPEG: scan for SOF markers
    if data.len() >= 2 && data[0] == 0xFF && data[1] == 0xD8 {
        let mut i = 2usize;
        while i + 8 < data.len() {
            if data[i] != 0xFF {
                break;
            }
            let marker = data[i + 1];
            let seg_len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
            // SOF markers: 0xC0-0xC3, 0xC5-0xC7, 0xC9-0xCB, 0xCD-0xCF
            if (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xC8 {
                if i + 8 < data.len() {
                    let h = u16::from_be_bytes([data[i + 5], data[i + 6]]) as u32;
                    let w = u16::from_be_bytes([data[i + 7], data[i + 8]]) as u32;
                    return Some((w, h));
                }
            }
            i += 2 + seg_len;
        }
    }
    // GIF
    if data.len() >= 10 && (data.starts_with(b"GIF89a") || data.starts_with(b"GIF87a")) {
        let w = u16::from_le_bytes([data[6], data[7]]) as u32;
        let h = u16::from_le_bytes([data[8], data[9]]) as u32;
        return Some((w, h));
    }
    // WEBP
    if data.len() >= 16 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        let vp8_type = &data[12..16];
        if vp8_type == b"VP8X" && data.len() >= 30 {
            let w = ((data[26] as u32) << 16 | (data[25] as u32) << 8 | data[24] as u32) + 1;
            let h = ((data[29] as u32) << 16 | (data[28] as u32) << 8 | data[27] as u32) + 1;
            return Some((w, h));
        } else if vp8_type == b"VP8L" && data.len() >= 25 {
            if data[20] == 0x2f {
                let b1 = data[21];
                let b2 = data[22];
                let b3 = data[23];
                let b4 = data[24];
                let w = (((b2 as u32 & 0x3F) << 8) | b1 as u32) + 1;
                let h = ((((b4 as u32 & 0xF) << 10) | ((b3 as u32) << 2) | ((b2 as u32 & 0xC0) >> 6)) & 0x3FFF) + 1;
                return Some((w, h));
            }
        } else if vp8_type == b"VP8 " && data.len() >= 30 {
            let w = u16::from_le_bytes([data[26], data[27]]) & 0x3FFF;
            let h = u16::from_le_bytes([data[28], data[29]]) & 0x3FFF;
            return Some((w as u32, h as u32));
        }
    }
    None
}

fn generate_random_fp(impersonate_list: &[String], user_agents_list: &[String]) -> (String, String, String, Option<String>, Option<String>, Option<String>) {
    let mut rng = rand::thread_rng();
    
    let ua = if !user_agents_list.is_empty() {
        user_agents_list.choose(&mut rng).unwrap().clone()
    } else {
        let chrome_versions = [120, 121, 122, 123, 124];
        let chrome_ver = chrome_versions.choose(&mut rng).unwrap();
        let is_mac = rng.gen_bool(0.5);
        if is_mac {
            format!("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{}.0.0.0 Safari/537.36", chrome_ver)
        } else {
            format!("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{}.0.0.0 Safari/537.36", chrome_ver)
        }
    };

    let imp = if !impersonate_list.is_empty() {
        impersonate_list.choose(&mut rng).unwrap().clone()
    } else {
        let default_imps = ["chrome119", "chrome120", "chrome123", "safari15_3", "edge101"];
        default_imps.choose(&mut rng).unwrap().to_string()
    };

    let dev_id = Uuid::new_v4().to_string();

    let mut sec_ch_ua = None;
    let mut sec_ch_ua_platform = None;
    let sec_ch_ua_mobile = Some("?0".to_string());

    if ua.contains("Chrome/") {
        let version = ua.split("Chrome/").nth(1).and_then(|s| s.split('.').next()).unwrap_or("120");
        sec_ch_ua = Some(format!("\"Not_A Brand\";v=\"8\", \"Chromium\";v=\"{0}\", \"Google Chrome\";v=\"{0}\"", version));
        if ua.contains("Windows") {
            sec_ch_ua_platform = Some("\"Windows\"".to_string());
        } else if ua.contains("Macintosh") {
            sec_ch_ua_platform = Some("\"macOS\"".to_string());
        }
    } else if ua.contains("Edge/") || ua.contains("Edg/") {
        let version = ua.split("Edg/").nth(1).or_else(|| ua.split("Edge/").nth(1)).and_then(|s| s.split('.').next()).unwrap_or("120");
        sec_ch_ua = Some(format!("\"Not_A Brand\";v=\"8\", \"Chromium\";v=\"{0}\", \"Microsoft Edge\";v=\"{0}\"", version));
        if ua.contains("Windows") {
            sec_ch_ua_platform = Some("\"Windows\"".to_string());
        } else if ua.contains("Macintosh") {
            sec_ch_ua_platform = Some("\"macOS\"".to_string());
        }
    }

    (ua, imp, dev_id, sec_ch_ua, sec_ch_ua_platform, sec_ch_ua_mobile)
}

impl ChatService {
    pub async fn new(
        state: AppState,
        config: Config,
        origin_token: Option<String>,
        data: &Value,
    ) -> Result<Self, actix_web::Error> {
        let req_token = get_req_token(
            &state,
            &config,
            origin_token.as_deref().unwrap_or(""),
            data.get("seed").and_then(|v| v.as_str()),
        )
        .await?;

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

        // 从请求体获取 account_id override
        if let Some(acc_id_override) = data.get("Chatgpt-Account-Id").and_then(|v| v.as_str()) {
            account_id = Some(acc_id_override.to_string());
        }

        let mut impersonate = "safari15_3".to_string();
        let mut user_agent = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/15.3 Safari/605.1.15".to_string();
        let mut oai_device_id = Uuid::new_v4().to_string();
        let mut sec_ch_ua: Option<String> = None;
        let mut sec_ch_ua_platform: Option<String> = None;
        let mut sec_ch_ua_mobile: Option<String> = None;

        if !req_token.is_empty() {
            let mut inner = state.inner.write().await;
            if let Some(fp_val) = inner.fp_map.get(&req_token) {
                if let Some(ua) = fp_val.get("user-agent").and_then(|v| v.as_str()) {
                    user_agent = ua.to_string();
                }
                if let Some(imp) = fp_val.get("impersonate").and_then(|v| v.as_str()) {
                    impersonate = imp.to_string();
                }
                if let Some(dev_id) = fp_val.get("oai-device-id").and_then(|v| v.as_str()) {
                    oai_device_id = dev_id.to_string();
                }
                sec_ch_ua = fp_val.get("sec-ch-ua").and_then(|v| v.as_str()).map(|s| s.to_string());
                sec_ch_ua_platform = fp_val.get("sec-ch-ua-platform").and_then(|v| v.as_str()).map(|s| s.to_string());
                sec_ch_ua_mobile = fp_val.get("sec-ch-ua-mobile").and_then(|v| v.as_str()).map(|s| s.to_string());
            } else {
                let (new_ua, new_imp, new_dev_id, sec_ch_ua_val, sec_ch_ua_plat_val, sec_ch_ua_mob_val) = 
                    generate_random_fp(&inner.impersonate_list, &config.user_agents_list);

                let mut fp_obj = serde_json::Map::new();
                fp_obj.insert("user-agent".to_string(), json!(new_ua));
                fp_obj.insert("impersonate".to_string(), json!(new_imp));
                fp_obj.insert("oai-device-id".to_string(), json!(new_dev_id));
                if let Some(ref val) = sec_ch_ua_val {
                    fp_obj.insert("sec-ch-ua".to_string(), json!(val));
                }
                if let Some(ref val) = sec_ch_ua_plat_val {
                    fp_obj.insert("sec-ch-ua-platform".to_string(), json!(val));
                }
                if let Some(ref val) = sec_ch_ua_mob_val {
                    fp_obj.insert("sec-ch-ua-mobile".to_string(), json!(val));
                }

                user_agent = new_ua;
                impersonate = new_imp;
                oai_device_id = new_dev_id;
                sec_ch_ua = sec_ch_ua_val;
                sec_ch_ua_platform = sec_ch_ua_plat_val;
                sec_ch_ua_mobile = sec_ch_ua_mob_val;

                inner.fp_map.insert(req_token.clone(), Value::Object(fp_obj));
                drop(inner);
                state.save_fp_map().await;
            }
        }

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

        let client = create_client(main_proxy.as_deref(), &impersonate)
            .map_err(|e| ErrorInternalServerError(format!("Failed to create client: {:?}", e)))?;
        let sentinel_client = create_client(sentinel_proxy.as_deref(), &impersonate)
            .map_err(|e| ErrorInternalServerError(format!("Failed to create sentinel client: {:?}", e)))?;

        let mut base_headers = HeaderMap::new();
        base_headers.insert("accept", HeaderValue::from_static("*/*"));
        base_headers.insert("accept-encoding", HeaderValue::from_static("gzip, deflate, br, zstd"));
        base_headers.insert("accept-language", HeaderValue::from_static("en-US,en;q=0.9"));
        base_headers.insert("content-type", HeaderValue::from_static("application/json"));
        base_headers.insert("oai-device-id", HeaderValue::from_str(&oai_device_id).unwrap());
        base_headers.insert("oai-language", HeaderValue::from_str(&config.oai_language).unwrap());
        base_headers.insert("origin", HeaderValue::from_str(&host_url).unwrap());
        base_headers.insert("priority", HeaderValue::from_static("u=1, i"));
        base_headers.insert("referer", HeaderValue::from_str(&format!("{}/", host_url)).unwrap());
        base_headers.insert("sec-fetch-dest", HeaderValue::from_static("empty"));
        base_headers.insert("sec-fetch-mode", HeaderValue::from_static("cors"));
        base_headers.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
        base_headers.insert("user-agent", HeaderValue::from_str(&user_agent).unwrap());

        if let Some(ref val) = sec_ch_ua {
            if let Ok(hv) = HeaderValue::from_str(val) {
                base_headers.insert("sec-ch-ua", hv);
            }
        }
        if let Some(ref val) = sec_ch_ua_platform {
            if let Ok(hv) = HeaderValue::from_str(val) {
                base_headers.insert("sec-ch-ua-platform", hv);
            }
        }
        if let Some(ref val) = sec_ch_ua_mobile {
            if let Ok(hv) = HeaderValue::from_str(val) {
                base_headers.insert("sec-ch-ua-mobile", hv);
            }
        }

        let base_url = if access_token.is_some() {
            base_headers.insert(
                "authorization",
                HeaderValue::from_str(&format!("Bearer {}", access_token.as_ref().unwrap())).unwrap(),
            );
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

        // 模型解析
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

        let resp_model = model_map
            .get(origin_model.as_str())
            .cloned()
            .unwrap_or(origin_model.as_str())
            .to_string();

        let gizmo_id = if origin_model.contains("gizmo") || origin_model.contains("g-") {
            origin_model.split("g-").last().map(|s| format!("g-{}", s))
        } else {
            None
        };

        let req_model = if origin_model.contains("o3-mini-high") {
            "o3-mini-high"
        } else if origin_model.contains("o3-mini-medium") {
            "o3-mini-medium"
        } else if origin_model.contains("o3-mini-low") {
            "o3-mini-low"
        } else if origin_model.contains("o3-mini") {
            "o3-mini"
        } else if origin_model.contains("o3") {
            "o3"
        } else if origin_model.contains("o1-preview") {
            "o1-preview"
        } else if origin_model.contains("o1-pro") {
            "o1-pro"
        } else if origin_model.contains("o1-mini") {
            "o1-mini"
        } else if origin_model.contains("o1") {
            "o1"
        } else if origin_model.contains("gpt-4.5o") {
            "gpt-4.5o"
        } else if origin_model.contains("gpt-4o-canmore") {
            "gpt-4o-canmore"
        } else if origin_model.contains("gpt-4o-mini") {
            "gpt-4o-mini"
        } else if origin_model.contains("gpt-4o") {
            "gpt-4o"
        } else if origin_model.contains("gpt-4-mobile") {
            "gpt-4-mobile"
        } else if origin_model.contains("gpt-4") {
            "gpt-4"
        } else if origin_model.contains("gpt-3.5") {
            "text-davinci-002-render-sha"
        } else if origin_model.contains("auto") {
            "auto"
        } else {
            "gpt-4o"
        }
        .to_string();

        let history_disabled = data
            .get("history_disabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(config.history_disabled);

        let conversation_id = data.get("conversation_id").and_then(|v| v.as_str()).map(|s| s.to_string());
        let parent_message_id = data.get("parent_message_id").and_then(|v| v.as_str()).map(|s| s.to_string());
        let max_tokens = data
            .get("max_tokens")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(usize::MAX);

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
            conversation_id,
            parent_message_id,
            max_tokens,
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

        let resp_res = self.client.get(&self.host_url).headers(self.base_headers.clone()).send().await;

        if let Ok(resp) = resp_res {
            if resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                if let Some(caps) = regex::Regex::new(r#"data-build="([^"]*)""#).unwrap().captures(&body) {
                    let dpl = caps.get(1).map(|m: regex::Match| m.as_str().to_string()).unwrap_or_default();
                    *CACHED_DPL.lock().unwrap() = Some(dpl.clone());
                    *CACHED_SCRIPT.lock().unwrap() =
                        Some("https://chatgpt.com/backend-api/sentinel/sdk.js".to_string());
                    *CACHED_TIME.lock().unwrap() = now;
                    info!("Found DPL: {:?}", dpl);
                }
            }
        }

        {
            let mut cached_dpl = CACHED_DPL.lock().unwrap();
            if cached_dpl.is_none() {
                *cached_dpl = Some("prod-f501fe933b3edf57aea882da888e1a544df99840".to_string());
                *CACHED_SCRIPT.lock().unwrap() =
                    Some("https://chatgpt.com/backend-api/sentinel/sdk.js".to_string());
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

        let resp_res = self.sentinel_client.post(&url).headers(self.base_headers.clone()).json(&payload).send().await;

        match resp_res {
            Ok(resp) => {
                let status = resp.status();
                info!("Sentinel response status: {}", status);
                if status.is_success() {
                    let text = resp.text().await.unwrap_or_default();
                    info!("Sentinel response body: {}", text);
                    let json_val: Value = serde_json::from_str(&text).unwrap_or(Value::Null);

                    self.chat_token =
                        json_val.get("token").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    if self.chat_token.is_empty() {
                        return Err(ErrorForbidden(format!(
                            "Failed to get chat requirements token, body was: {}",
                            serde_json::to_string(&json_val).unwrap_or_default()
                        )));
                    }

                    // 校验模型权限
                    let persona = json_val.get("persona").and_then(|v| v.as_str()).unwrap_or("");
                    if persona != "chatgpt-paid"
                        && (self.req_model == "gpt-4" || self.req_model == "o1-preview")
                    {
                        return Err(ErrorNotFound(
                            json!({
                                "message": format!("The model `{}` does not exist or you do not have access to it.", self.origin_model),
                                "type": "invalid_request_error",
                                "code": "model_not_found"
                            })
                            .to_string(),
                        ));
                    }

                    // 处理 Turnstile
                    if let Some(turnstile) = json_val.get("turnstile") {
                        if turnstile.get("required").and_then(|v| v.as_bool()).unwrap_or(false) {
                            if let Some(dx) = turnstile.get("dx").and_then(|v| v.as_str()) {
                                // 优先使用远程 turnstile_solver_url（与 Python 对齐）
                                if let Some(ref solver_url) = self.config.turnstile_solver_url.clone() {
                                    let payload = json!({
                                        "url": "https://chatgpt.com",
                                        "p": p_token,
                                        "dx": dx,
                                        "ua": self.user_agent
                                    });
                                    match self.client.post(solver_url).json(&payload).send().await {
                                        Ok(r) => {
                                            if let Ok(r_json) = r.json::<Value>().await {
                                                if let Some(t) = r_json.get("t").and_then(|v| v.as_str()) {
                                                    self.turnstile_token = Some(t.to_string());
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            info!("Turnstile solver ignored: {:?}", e);
                                        }
                                    }
                                } else {
                                    // fallback: 本地处理
                                    self.turnstile_token = Some(process_turnstile(dx, &p_token));
                                }
                            }
                        }
                    }

                    // 处理 Proof of Work
                    if let Some(pow) = json_val.get("proofofwork") {
                        if pow.get("required").and_then(|v| v.as_bool()).unwrap_or(false) {
                            let diff = pow
                                .get("difficulty")
                                .and_then(|v| v.as_str())
                                .unwrap_or("000032");
                            // 与 Python 对齐：如果难度 <= pow_difficulty 则拒绝
                            if diff <= self.config.pow_difficulty.as_str() {
                                return Err(ErrorForbidden(format!(
                                    "Proof of work difficulty too high: {}",
                                    diff
                                )));
                            }
                            let seed = pow.get("seed").and_then(|v| v.as_str()).unwrap_or("");
                            let (ans, solved) = get_answer_token(seed, diff, &config_val);
                            if solved {
                                self.proof_token = Some(ans);
                            } else {
                                return Err(ErrorForbidden("Failed to solve proof of work"));
                            }
                        }
                    }

                    // 处理 Arkose
                    let arkose_opt = json_val.get("arkose").or_else(|| json_val.get("ark0se"));
                    if let Some(arkose) = arkose_opt {
                        if arkose.get("required").and_then(|v| v.as_bool()).unwrap_or(false) {
                            let method = if persona == "chatgpt-freeaccount" {
                                "chat35"
                            } else {
                                "chat4"
                            };
                            if self.config.ark0se_token_url_list.is_empty() {
                                return Err(ErrorForbidden("Arkose service required"));
                            }
                            let ark0se_dx = arkose.get("dx").and_then(|v| v.as_str()).unwrap_or("");
                            
                            let mut rng = rand::thread_rng();
                            let ark0se_token_url = self.config.ark0se_token_url_list.choose(&mut rng).unwrap();

                            let ark0se_client = create_client(self.config.proxy_url_list.choose(&mut rng).map(|s| s.as_str()), &self.impersonate)
                                .map_err(|e| ErrorInternalServerError(format!("Failed to create arkose client: {:?}", e)))?;

                            let payload = json!({
                                "blob": ark0se_dx,
                                "method": method
                            });

                            let resp_res = ark0se_client.post(ark0se_token_url).json(&payload).send().await;
                            match resp_res {
                                Ok(r) => {
                                    if r.status().is_success() {
                                        if let Ok(r_json) = r.json::<Value>().await {
                                            let solved = r_json.get("solved").and_then(|v| v.as_bool()).unwrap_or(true);
                                            if solved {
                                                if let Some(t) = r_json.get("token").and_then(|v| v.as_str()) {
                                                    self.ark0se_token = Some(t.to_string());
                                                } else {
                                                    return Err(ErrorForbidden("Failed to get Ark0se token (missing token)"));
                                                }
                                            } else {
                                                return Err(ErrorForbidden("Failed to get Ark0se token (not solved)"));
                                            }
                                        } else {
                                            return Err(ErrorForbidden("Failed to parse Ark0se token response"));
                                        }
                                    } else {
                                        return Err(ErrorForbidden(format!("Ark0se solver returned status: {}", r.status())));
                                    }
                                }
                                Err(e) => {
                                    return Err(ErrorForbidden(format!("Failed to request Ark0se token: {:?}", e)));
                                }
                            }
                        }
                    }
                    Ok(())
                } else {
                    let err_text = resp.text().await.unwrap_or_default();
                    // 与 Python 对齐：cf_chl_opt 和 rate-limit 特殊处理
                    if err_text.contains("cf_chl_opt") {
                        return Err(ErrorForbidden("cf_chl_opt"));
                    }
                    if status.as_u16() == 429 {
                        // 429 时将 token 加入错误列表
                        self.mark_token_error().await;
                        return Err(actix_web::error::ErrorTooManyRequests("rate-limit"));
                    }
                    error!("Sentinel non-200 status {}, body: {}", status, err_text);
                    Err(ErrorForbidden(format!(
                        "Sentinel returns status {}, detail: {}",
                        status, err_text
                    )))
                }
            }
            Err(e) => {
                error!("Sentinel request error: {:?}", e);
                Err(ErrorInternalServerError(format!("Sentinel handshake failed: {:?}", e)))
            }
        }
    }

    /// 将当前 token 加入错误列表（对齐 Python check_is_limit）
    async fn mark_token_error(&self) {
        if self.req_token.is_empty() {
            return;
        }
        let mut inner = self.state.inner.write().await;
        if !inner.error_token_list.contains(&self.req_token) {
            inner.error_token_list.push(self.req_token.clone());
            // 持久化
            use std::fs;
            let file = "data/error_token.txt";
            if let Ok(mut content) = fs::read_to_string(file) {
                if !content.ends_with('\n') && !content.is_empty() {
                    content.push('\n');
                }
                content.push_str(&self.req_token);
                content.push('\n');
                let _ = fs::write(file, content);
            } else {
                let _ = fs::write(file, format!("{}\n", self.req_token));
            }
        }
    }

    // 拼装请求 Body（与 Python prepare_send_conversation 对齐，随机 client_contextual_info）
    pub async fn prepare_send_conversation(&self, chat_messages: Value, parent_message_id: Option<&str>) -> Value {
        let mut rng = rand::thread_rng();
        let conversation_mode = if let Some(ref gid) = self.gizmo_id {
            info!("Gizmo id: {}", gid);
            json!({ "kind": "gizmo_interaction", "gizmo_id": gid })
        } else {
            json!({ "kind": "primary_assistant" })
        };

        info!("Model mapping: {} -> {}", self.origin_model, self.req_model);

        let mut req_body = json!({
            "action": "next",
            "client_contextual_info": {
                "is_dark_mode": false,
                "time_since_loaded": rng.gen_range(50..500i64),
                "page_height": rng.gen_range(500..1000i64),
                "page_width": rng.gen_range(1000..2000i64),
                "pixel_ratio": 1.5,
                "screen_height": rng.gen_range(800..1200i64),
                "screen_width": rng.gen_range(1200..2200i64)
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
            "parent_message_id": parent_message_id
                .or(self.parent_message_id.as_deref())
                .map(|s| s.to_string())
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            "reset_rate_limits": false,
            "suggestions": [],
            "supported_encodings": [],
            "system_hints": [],
            "timezone": "America/Los_Angeles",
            "timezone_offset_min": -480,
            "variant_purpose": "comparison_implicit",
            "websocket_request_id": Uuid::new_v4().to_string()
        });

        // 如果有 conversation_id，加入请求体
        if let Some(ref conv_id) = self.conversation_id {
            req_body.as_object_mut().unwrap().insert("conversation_id".to_string(), json!(conv_id));
        }

        req_body
    }

    // 发送会话请求
    pub async fn send_conversation_request(&self, req_body: Value) -> Result<rquest::Response, actix_web::Error> {
        let url = format!("{}/conversation", self.base_url);
        let mut headers = self.base_headers.clone();
        headers.insert("accept", HeaderValue::from_static("text/event-stream"));
        headers.insert("accept-encoding", HeaderValue::from_static("identity"));

        if !self.config.conversation_only {
            if let Ok(hv) = HeaderValue::from_str(&self.chat_token) {
                headers.insert("openai-sentinel-chat-requirements-token", hv);
            }
            if let Some(ref proof) = self.proof_token {
                if let Ok(hv) = HeaderValue::from_str(proof) {
                    headers.insert("openai-sentinel-proof-token", hv);
                }
            }
            if let Some(ref ark) = self.ark0se_token {
                if let Ok(hv) = HeaderValue::from_str(ark) {
                    headers.insert("openai-sentinel-arkose-token", hv);
                }
            }
            if let Some(ref turnstile) = self.turnstile_token {
                if let Ok(hv) = HeaderValue::from_str(turnstile) {
                    headers.insert("openai-sentinel-turnstile-token", hv);
                }
            }
        }

        info!("Sending conversation request to: {}", url);
        info!("Request body: {}", serde_json::to_string(&req_body).unwrap_or_default());

        let resp_res = self.client.post(&url).headers(headers).json(&req_body).send().await;

        match resp_res {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    Ok(resp)
                } else {
                    let err_text = resp.text().await.unwrap_or_default();
                    if err_text.contains("cf_chl_opt") {
                        return Err(ErrorForbidden("cf_chl_opt"));
                    }
                    if status.as_u16() == 429 {
                        self.mark_token_error().await;
                        return Err(actix_web::error::ErrorTooManyRequests("rate-limit"));
                    }
                    Err(ErrorInternalServerError(format!(
                        "OpenAI conversation failed, status {}, detail: {}",
                        status, err_text
                    )))
                }
            }
            Err(e) => Err(ErrorInternalServerError(format!(
                "Request to OpenAI conversation failed: {:?}",
                e
            ))),
        }
    }

    // 辅助多模态文件下载
    pub async fn get_file_content_from_url(&self, url: &str) -> Result<(Vec<u8>, String), actix_web::Error> {
        if url.starts_with("data:") {
            let parts: Vec<&str> = url.splitn(2, ',').collect();
            if parts.len() == 2 {
                let header = parts[0];
                let base64_data = parts[1];
                let mime_type = header
                    .split(';')
                    .next()
                    .and_then(|s| s.split(':').nth(1))
                    .unwrap_or("image/png")
                    .to_string();
                
                if let Ok(decoded_bytes) = base64::Engine::decode(&base64::prelude::BASE64_STANDARD, base64_data) {
                    return Ok((decoded_bytes, mime_type));
                }
            }
            return Err(ErrorBadRequest("Failed to decode data URL base64"));
        }

        let mut builder = rquest::Client::builder()
            .impersonate(rquest::tls::Impersonate::Safari15_3)
            .danger_accept_invalid_certs(true);

        if let Some(ref export_proxy) = self.config.export_proxy_url {
            if !export_proxy.is_empty() {
                if let Ok(proxy) = rquest::Proxy::all(export_proxy) {
                    builder = builder.proxy(proxy);
                }
            }
        }
        let download_client = builder.build()
            .map_err(|e| ErrorInternalServerError(format!("Failed to create download client: {:?}", e)))?;

        let (bytes, mime) = if let Some(ref cf_url) = self.config.cf_file_url {
            if !cf_url.is_empty() {
                let body = json!({ "file_url": url });
                let resp = download_client
                    .post(cf_url)
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| ErrorBadRequest(format!("Failed to fetch file URL via cf_file_url: {:?}", e)))?;
                let mime = resp
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("application/octet-stream")
                    .split(';')
                    .next()
                    .unwrap_or("application/octet-stream")
                    .trim()
                    .to_string();
                let bytes = resp
                    .bytes()
                    .await
                    .map_err(|e| ErrorBadRequest(format!("Failed to read file bytes from cf_file_url: {:?}", e)))?;
                (bytes.to_vec(), mime)
            } else {
                let resp = download_client
                    .get(url)
                    .send()
                    .await
                    .map_err(|e| ErrorBadRequest(format!("Failed to fetch file URL: {:?}", e)))?;
                let mime = resp
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("application/octet-stream")
                    .split(';')
                    .next()
                    .unwrap_or("application/octet-stream")
                    .trim()
                    .to_string();
                let bytes = resp
                    .bytes()
                    .await
                    .map_err(|e| ErrorBadRequest(format!("Failed to read file bytes: {:?}", e)))?;
                (bytes.to_vec(), mime)
            }
        } else {
            let resp = download_client
                .get(url)
                .send()
                .await
                .map_err(|e| ErrorBadRequest(format!("Failed to fetch file URL: {:?}", e)))?;
            let mime = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/octet-stream")
                .split(';')
                .next()
                .unwrap_or("application/octet-stream")
                .trim()
                .to_string();
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| ErrorBadRequest(format!("Failed to read file bytes: {:?}", e)))?;
            (bytes.to_vec(), mime)
        };

        Ok((bytes, mime))
    }

    /// 申请上传 URL（对齐 Python get_upload_url）
    async fn get_upload_url(
        &self,
        file_name: &str,
        file_size: usize,
        use_case: &str,
    ) -> Result<(String, String), actix_web::Error> {
        let url = format!("{}/files", self.base_url);
        let resp = self
            .client
            .post(&url)
            .headers(self.base_headers.clone())
            .json(&json!({
                "file_name": file_name,
                "file_size": file_size,
                "reset_rate_limits": false,
                "timezone_offset_min": -480,
                "use_case": use_case
            }))
            .send()
            .await
            .map_err(|e| ErrorInternalServerError(format!("get_upload_url request failed: {:?}", e)))?;

        if resp.status().is_success() {
            let res: Value = resp
                .json()
                .await
                .map_err(|e| ErrorInternalServerError(format!("get_upload_url parse failed: {:?}", e)))?;
            let file_id = res.get("file_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let upload_url = res.get("upload_url").and_then(|v| v.as_str()).unwrap_or("").to_string();
            info!("file_id: {}, upload_url: {}", file_id, upload_url);
            Ok((file_id, upload_url))
        } else {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            Err(ErrorInternalServerError(format!("get_upload_url failed: {} {}", status, text)))
        }
    }

    /// PUT 上传文件内容（对齐 Python upload）
    async fn upload_content(
        &self,
        upload_url: &str,
        file_content: &[u8],
        mime_type: &str,
    ) -> Result<bool, actix_web::Error> {
        let mut headers = self.base_headers.clone();
        headers.insert("accept", HeaderValue::from_static("application/json, text/plain, */*"));
        if let Ok(hv) = HeaderValue::from_str(mime_type) {
            headers.insert("content-type", hv);
        }
        headers.insert("x-ms-blob-type", HeaderValue::from_static("BlockBlob"));
        headers.insert("x-ms-version", HeaderValue::from_static("2020-04-08"));
        headers.remove("authorization");
        headers.remove("oai-device-id");
        headers.remove("oai-language");

        let resp = self
            .client
            .put(upload_url)
            .headers(headers)
            .body(file_content.to_vec())
            .send()
            .await
            .map_err(|e| ErrorInternalServerError(format!("upload_content failed: {:?}", e)))?;

        Ok(resp.status().as_u16() == 201)
    }

    /// 确认上传完成，获取 download_url（对齐 Python get_download_url_from_upload）
    async fn get_download_url_from_upload(&self, file_id: &str) -> Option<String> {
        let url = format!("{}/files/{}/uploaded", self.base_url, file_id);
        let resp = self
            .client
            .post(&url)
            .headers(self.base_headers.clone())
            .json(&json!({}))
            .send()
            .await
            .ok()?;
        if resp.status().is_success() {
            let res: Value = resp.json().await.ok()?;
            res.get("download_url").and_then(|v| v.as_str()).map(|s| s.to_string())
        } else {
            None
        }
    }

    /// 完整文件上传流程（对齐 Python upload_file）
    pub async fn upload_file(
        &self,
        file_content: &[u8],
        mime_type: &str,
    ) -> Result<Option<FileMeta>, actix_web::Error> {
        if file_content.is_empty() || mime_type.is_empty() {
            return Ok(None);
        }

        let mut actual_mime = mime_type.to_string();
        let mut width: Option<u32> = None;
        let mut height: Option<u32> = None;

        if mime_type.starts_with("image/") {
            match get_image_size(file_content) {
                Some((w, h)) => {
                    width = Some(w);
                    height = Some(h);
                }
                None => {
                    // 无法解析尺寸，降级为 text/plain（与 Python 对齐）
                    actual_mime = "text/plain".to_string();
                }
            }
        }

        let file_size = file_content.len();
        let file_extension = get_file_extension(&actual_mime);
        let file_name = format!("{}{}", Uuid::new_v4(), file_extension);
        let use_case = determine_file_use_case(&actual_mime);

        let (file_id, upload_url) = match self.get_upload_url(&file_name, file_size, use_case).await {
            Ok((id, url)) if !id.is_empty() && !url.is_empty() => (id, url),
            _ => return Ok(None),
        };

        if !self.upload_content(&upload_url, file_content, &actual_mime).await.unwrap_or(false) {
            return Ok(None);
        }

        let _download_url = self.get_download_url_from_upload(&file_id).await;
        // download_url 仅用于验证，实际引用 file_id

        let meta = FileMeta {
            file_id,
            size_bytes: file_size,
            file_name,
            mime_type: actual_mime,
            use_case: use_case.to_string(),
            width,
            height,
        };
        info!("File_meta: file_id={}, size={}, use_case={}", meta.file_id, meta.size_bytes, meta.use_case);
        Ok(Some(meta))
    }

    /// 轮询确认文档索引完成（对齐 Python check_upload）
    pub async fn check_upload(&self, file_id: &str) -> bool {
        let url = format!("{}/files/{}", self.base_url, file_id);
        for _ in 0..30 {
            if let Ok(resp) = self.client.get(&url).headers(self.base_headers.clone()).send().await {
                if resp.status().is_success() {
                    if let Ok(res) = resp.json::<Value>().await {
                        if res
                            .get("retrieval_index_status")
                            .and_then(|v| v.as_str())
                            == Some("success")
                        {
                            return true;
                        }
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        false
    }

    /// 获取已生成文件的下载 URL（对齐 Python get_download_url）
    pub async fn get_download_url(&self, file_id: &str) -> Option<String> {
        let url = format!("{}/files/{}/download", self.base_url, file_id);
        let resp = self
            .client
            .get(&url)
            .headers(self.base_headers.clone())
            .send()
            .await
            .ok()?;
        if resp.status().is_success() {
            let res: Value = resp.json().await.ok()?;
            res.get("download_url").and_then(|v| v.as_str()).map(|s| s.to_string())
        } else {
            error!("get_download_url failed: {}", resp.status());
            None
        }
    }

    /// 获取附件下载 URL（对齐 Python get_attachment_url）
    pub async fn get_attachment_url(&self, file_id: &str, conversation_id: &str) -> Option<String> {
        let url = format!(
            "{}/conversation/{}/attachment/{}/download",
            self.base_url, conversation_id, file_id
        );
        let resp = self
            .client
            .get(&url)
            .headers(self.base_headers.clone())
            .send()
            .await
            .ok()?;
        if resp.status().is_success() {
            let res: Value = resp.json().await.ok()?;
            res.get("download_url").and_then(|v| v.as_str()).map(|s| s.to_string())
        } else {
            error!("get_attachment_url failed: {}", resp.status());
            None
        }
    }

    /// 获取沙盒文件下载 URL（对齐 Python get_response_file_url）
    pub async fn get_response_file_url(
        &self,
        conversation_id: &str,
        message_id: &str,
        sandbox_path: &str,
    ) -> Option<String> {
        let url = format!("{}/conversation/{}/interpreter/download", self.base_url, conversation_id);
        let resp = self
            .client
            .get(&url)
            .headers(self.base_headers.clone())
            .query(&[("message_id", message_id), ("sandbox_path", sandbox_path)])
            .send()
            .await
            .ok()?;
        if resp.status().is_success() {
            let res: Value = resp.json().await.ok()?;
            res.get("download_url").and_then(|v| v.as_str()).map(|s| s.to_string())
        } else {
            info!("Failed to get response file url");
            None
        }
    }
}
