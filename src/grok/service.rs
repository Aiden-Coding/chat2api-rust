use serde_json::{json, Value};
use crate::config::Config;
use crate::globals::AppState;
use crate::grok::stream::{GrokStream, GrokStreamMode, format_not_stream_response};
use actix_web::{HttpResponse, web, error::{ErrorInternalServerError, ErrorUnauthorized}};
use log::{info, warn, error};
use rand::seq::SliceRandom;


pub fn build_console_payload(
    messages: &Value,
    model: &str,
    temperature: f64,
    top_p: f64,
    reasoning_effort: Option<&str>,
    stream: bool,
) -> Value {
    let mut input_items = Vec::new();

    if let Some(arr) = messages.as_array() {
        for msg in arr {
            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            let content = msg.get("content").unwrap_or(&Value::Null);

            let api_role = match role {
                "system" | "developer" => "system",
                "assistant" => "assistant",
                _ => "user",
            };

            let mut content_blocks = Vec::new();

            match content {
                Value::String(s) => {
                    content_blocks.push(json!({
                        "type": "input_text",
                        "text": s
                    }));
                }
                Value::Array(blocks) => {
                    for block in blocks {
                        if let Some(block_obj) = block.as_object() {
                            let btype = block_obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            if btype == "text" {
                                let text = block_obj.get("text").and_then(|v| v.as_str()).unwrap_or("");
                                content_blocks.push(json!({
                                    "type": "input_text",
                                    "text": text
                                }));
                            } else if btype == "image_url" {
                                let url = block_obj.get("image_url")
                                    .and_then(|v| v.get("url"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                if !url.is_empty() {
                                    content_blocks.push(json!({
                                        "type": "input_image",
                                        "image_url": url
                                    }));
                                }
                            } else {
                                let text = block_obj.get("text").and_then(|v| v.as_str())
                                    .unwrap_or_else(|| block.as_str().unwrap_or(""));
                                content_blocks.push(json!({
                                    "type": "input_text",
                                    "text": text
                                }));
                            }
                        }
                    }
                }
                _ => {
                    let s = content.as_str().unwrap_or("");
                    content_blocks.push(json!({
                        "type": "input_text",
                        "text": s
                    }));
                }
            }

            if !content_blocks.is_empty() {
                input_items.push(json!({
                    "role": api_role,
                    "content": content_blocks
                }));
            }
        }
    }

    let console_model = match model {
        // Grok 4.3 Series
        "grok-4.3-console" | "grok-4.3-low" | "grok-4.3-medium" | "grok-4.3-high" | 
        "grok-4.3-beta" | "grok-4.3" | "grok-beta" | "grok-3" | "grok-2" => "grok-4.3",
        
        // Grok 4.20 Reasoning Series
        "grok-4.20-0309-reasoning-console" | "grok-4.20-0309-reasoning" | 
        "grok-4.20-0309-reasoning-super" | "grok-4.20-0309-reasoning-heavy" | 
        "grok-4.20-expert" => "grok-4.20-0309-reasoning",
        
        // Grok 4.20 Auto Series
        "grok-4.20-0309-console" | "grok-4.20-0309" | 
        "grok-4.20-0309-super" | "grok-4.20-0309-heavy" | 
        "grok-4.20-auto" => "grok-4.20-0309",
        
        // Grok 4.20 Non-Reasoning Series
        "grok-4.20-0309-non-reasoning-console" | "grok-4.20-0309-non-reasoning" | 
        "grok-4.20-0309-non-reasoning-super" | "grok-4.20-0309-non-reasoning-heavy" | 
        "grok-4.20-fast" => "grok-4.20-0309-non-reasoning",
        
        // Grok 4.20 Multi-Agent Series
        "grok-4.20-multi-agent-console" | "grok-4.20-multi-agent-low" | 
        "grok-4.20-multi-agent-medium" | "grok-4.20-multi-agent-high" | 
        "grok-4.20-multi-agent-xhigh" | "grok-4.20-multi-agent-0309" | 
        "grok-4.20-heavy" => "grok-4.20-multi-agent-0309",
        
        // Grok Build Series
        "grok-build-console" | "grok-build-0.1" => "grok-build-0.1",
        
        other => other,
    };

    let effort = if model.ends_with("-low") {
        "low"
    } else if model.ends_with("-medium") {
        "medium"
    } else if model.ends_with("-high") {
        "high"
    } else if model.ends_with("-xhigh") {
        "xhigh"
    } else {
        match reasoning_effort {
            Some("none") => "none",
            Some("minimal") | Some("low") => "low",
            Some("medium") => "medium",
            Some("high") => "high",
            Some("xhigh") => "xhigh",
            _ => "medium",
        }
    };

    let max_output_tokens = match console_model {
        "grok-4.20-multi-agent-0309" => 2_000_000,
        "grok-build-0.1" => 256_000,
        _ => 1_000_000,
    };

    let mut payload = json!({
        "model": console_model,
        "input": input_items,
        "max_output_tokens": max_output_tokens,
        "temperature": temperature,
        "top_p": top_p,
        "store": false,
        "include": ["reasoning.encrypted_content"],
        "stream": stream
    });

    let with_reasoning = console_model == "grok-4.3" || console_model == "grok-4.20-multi-agent-0309";
    if with_reasoning {
        payload.as_object_mut().unwrap().insert(
            "reasoning".to_string(),
            json!({ "effort": effort }),
        );
    }

    let with_search_tools = console_model == "grok-4.20-multi-agent-0309"
        || console_model == "grok-4.20-0309"
        || console_model == "grok-4.20-0309-reasoning"
        || console_model == "grok-4.20-0309-non-reasoning"
        || console_model == "grok-4.3"
        || console_model == "grok-build-0.1";

    if with_search_tools {
        payload.as_object_mut().unwrap().insert(
            "tools".to_string(),
            json!([
                {"type": "web_search", "enable_image_understanding": true},
                {"type": "x_search", "enable_video_understanding": true}
            ]),
        );
        payload.as_object_mut().unwrap().insert(
            "tool_choice".to_string(),
            json!("auto"),
        );
    }

    payload
}

pub async fn get_grok_req_token(
    state: &AppState,
    config: &Config,
    req_token: &str,
    exclude_tokens: &[String],
) -> Result<String, actix_web::Error> {
    let mut inner = state.inner.write().await;

    // Clean up expired rate-limited tokens
    let now = std::time::Instant::now();
    inner.grok_rate_limited_tokens.retain(|_, &mut expire_time| expire_time > now);

    if !req_token.is_empty() {
        let should_allocate = if config.authorization_list.is_empty() {
            false
        } else {
            config.authorization_list.contains(&req_token.to_string())
        };

        if !should_allocate {
            return Ok(req_token.to_string());
        }
    }

    let available_tokens: Vec<String> = inner.grok_token_list.iter()
        .filter(|t| {
            !inner.grok_error_token_list.contains(t)
            && !inner.grok_rate_limited_tokens.contains_key(*t)
            && !exclude_tokens.contains(t)
        })
        .cloned()
        .collect();

    if !available_tokens.is_empty() {
        if config.random_token {
            let mut rng = rand::thread_rng();
            let chosen = available_tokens.choose(&mut rng).unwrap().clone();
            return Ok(chosen);
        } else {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static GROK_ROBIN_COUNTER: AtomicUsize = AtomicUsize::new(0);
            let count = GROK_ROBIN_COUNTER.fetch_add(1, Ordering::Relaxed);
            let index = count % available_tokens.len();
            return Ok(available_tokens[index].clone());
        }
    }

    // Fallback to any active token if all are locked/excluded
    let fallback_tokens: Vec<String> = inner.grok_token_list.iter()
        .filter(|t| !inner.grok_error_token_list.contains(t))
        .cloned()
        .collect();
    if !fallback_tokens.is_empty() {
        return Ok(fallback_tokens[0].clone());
    }

    Ok(String::new())
}

fn is_console_model(model: &str) -> bool {
    model.contains("console")
        || model.ends_with("-low")
        || model.ends_with("-medium")
        || model.ends_with("-high")
        || model.ends_with("-xhigh")
        || model.starts_with("grok-build")
}

fn get_web_mode_id(model: &str) -> &'static str {
    match model {
        "grok-4.20-0309-non-reasoning" | "grok-4.20-0309-non-reasoning-super" | "grok-4.20-0309-non-reasoning-heavy" | "grok-4.20-fast" => "fast",
        "grok-4.3-fast" => "fast",  // Keep web routing for grok-4.3-fast
        "grok-4.20-0309" | "grok-4.20-0309-super" | "grok-4.20-0309-heavy" | "grok-4.20-auto" => "auto",
        "grok-4.20-0309-reasoning" | "grok-4.20-0309-reasoning-super" | "grok-4.20-0309-reasoning-heavy" | "grok-4.20-expert" => "expert",
        "grok-4.20-multi-agent-0309" | "grok-4.20-heavy" => "heavy",
        "grok-4.3-beta" | "grok-4.3" | "grok-beta" | "grok-3" | "grok-2" => "grok-420-computer-use-sa",
        _ => "grok-420-computer-use-sa",
    }
}

fn messages_to_web_prompt(messages: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(arr) = messages.as_array() {
        for msg in arr {
            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            let content = msg.get("content").unwrap_or(&Value::Null);

            let content_str = match content {
                Value::String(s) => s.clone(),
                Value::Array(blocks) => {
                    let mut text_blocks = Vec::new();
                    for block in blocks {
                        if let Some(block_obj) = block.as_object() {
                            let btype = block_obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            if btype == "text" {
                                let text = block_obj.get("text").and_then(|v| v.as_str()).unwrap_or("");
                                if !text.is_empty() {
                                    text_blocks.push(text.to_string());
                                }
                            }
                        }
                    }
                    text_blocks.join("\n")
                }
                _ => content.as_str().unwrap_or("").to_string(),
            };

            if !content_str.is_empty() {
                parts.push(format!("[{}]: {}", role, content_str));
            }
        }
    }
    parts.join("\n\n")
}

pub async fn handle_grok_conversation(
    origin_token: Option<String>,
    req_body: Value,
    state: web::Data<AppState>,
    config: web::Data<Config>,
) -> Result<HttpResponse, actix_web::Error> {
    let max_retries = config.retry_times;
    let mut last_err = None;

    let is_stream = req_body.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    let model = req_body.get("model").and_then(|v| v.as_str()).unwrap_or("grok-4.3").to_string();
    let temperature = req_body.get("temperature").and_then(|v| v.as_f64()).unwrap_or(0.7);
    let top_p = req_body.get("top_p").and_then(|v| v.as_f64()).unwrap_or(0.95);
    let reasoning_effort = req_body.get("reasoning_effort").and_then(|v| v.as_str());

    // Extract tool_names and inject tool system prompt
    let mut tool_names = Vec::new();
    let mut messages = req_body.get("messages").cloned().unwrap_or_else(|| serde_json::json!([]));
    
    if let Some(tools) = req_body.get("tools").and_then(|t| t.as_array()) {
        for tool in tools {
            if let Some(func) = tool.get("function").and_then(|f| f.as_object()) {
                if let Some(name) = func.get("name").and_then(|n| n.as_str()) {
                    let name_trimmed = name.trim();
                    if !name_trimmed.is_empty() {
                        tool_names.push(name_trimmed.to_string());
                    }
                }
            }
        }
        
        let tool_choice = req_body.get("tool_choice").unwrap_or(&serde_json::Value::Null);
        let tool_prompt = build_tool_system_prompt(tools, tool_choice);
        
        if let Some(arr) = messages.as_array_mut() {
            for msg in arr.iter_mut() {
                if let Some(obj) = msg.as_object_mut() {
                    let role = obj.get("role").and_then(|v| v.as_str()).unwrap_or("user");
                    if role == "assistant" {
                        if let Some(tcs) = obj.get("tool_calls").and_then(|v| v.as_array()) {
                            let xml = tool_calls_to_xml(tcs);
                            let content_str = obj.get("content").and_then(|v| v.as_str()).unwrap_or("").trim();
                            let new_content = if !content_str.is_empty() {
                                format!("[assistant]: {}\n{}", content_str, xml)
                            } else {
                                format!("[assistant]:\n{}", xml)
                            };
                            obj.insert("content".to_string(), json!(new_content));
                            obj.remove("tool_calls");
                        }
                    } else if role == "tool" {
                        let tool_call_id = obj.get("tool_call_id").and_then(|v| v.as_str()).unwrap_or("");
                        let content_str = obj.get("content").and_then(|v| v.as_str()).unwrap_or("").trim();
                        let label = if !tool_call_id.is_empty() {
                            format!("[tool result for {}]", tool_call_id)
                        } else {
                            "[tool result]".to_string()
                        };
                        let new_content = format!("{}:\n{}", label, content_str);
                        obj.insert("content".to_string(), json!(new_content));
                    }
                }
            }
            let sys_msg = json!({
                "role": "system",
                "content": tool_prompt
            });
            arr.insert(0, sys_msg);
        }
    }

    let mut excluded_tokens = Vec::new();

    for attempt in 0..=max_retries {
        if attempt > 0 {
            info!("正在重试发送 Grok 会话 (第 {}/{} 次重试)...", attempt, max_retries);
        }

        // Get SSO token
        let sso_token = match get_grok_req_token(
            &state,
            &config,
            origin_token.as_deref().unwrap_or(""),
            &excluded_tokens,
        ).await {
            Ok(tok) => tok,
            Err(e) => {
                error!("第 {} 次尝试失败: 获取 Grok Token 错误: {:?}", attempt, e);
                last_err = Some(e);
                continue;
            }
        };

        if sso_token.is_empty() {
            let err = ErrorUnauthorized(serde_json::json!({"error": "No SSO token available"}).to_string());
            last_err = Some(err);
            continue;
        }

        let clean_sso = if sso_token.starts_with("sso=") {
            &sso_token[4..]
        } else {
            &sso_token
        };

        // Select proxy
        let session_id = uuid::Uuid::new_v4().to_string();
        let main_proxy = if !config.proxy_url_list.is_empty() {
            let mut rng = rand::thread_rng();
            Some(config.proxy_url_list.choose(&mut rng).unwrap().replace("{}", &session_id))
        } else {
            None
        };

        let is_console = is_console_model(&model);

        // For console: use standard client (with invalid cert acceptance for proxy debug support)
        // For web (grok.com): use a dedicated client WITHOUT danger_accept_invalid_certs
        // to preserve the latest Chrome JA3/HTTP2 fingerprint supplied by rquest.
        let client = if is_console {
            let impersonate = {
                let inner = state.inner.read().await;
                let mut rng = rand::thread_rng();
                inner.impersonate_list.choose(&mut rng).cloned().unwrap_or_else(|| "chrome120".to_string())
            };
            match crate::chatgpt::client::create_client(main_proxy.as_deref(), &impersonate) {
                Ok(c) => c,
                Err(e) => {
                    let err = ErrorInternalServerError(format!("Failed to create client: {:?}", e));
                    last_err = Some(err);
                    continue;
                }
            }
        } else {
            match crate::chatgpt::client::create_grok_web_client(main_proxy.as_deref()) {
                Ok(c) => c,
                Err(e) => {
                    let err = ErrorInternalServerError(format!("Failed to create grok web client: {:?}", e));
                    last_err = Some(err);
                    continue;
                }
            }
        };
        
        let resp_res = if is_console {
            // Build request payload
            let payload = build_console_payload(
                &messages,
                &model,
                temperature,
                top_p,
                reasoning_effort,
                is_stream, // Use actual stream parameter from request
            );

            let mut headers = wreq::header::HeaderMap::new();
            headers.insert("accept", wreq::header::HeaderValue::from_static("*/*"));
            headers.insert("accept-encoding", wreq::header::HeaderValue::from_static("gzip, deflate, br, zstd"));
            headers.insert("accept-language", wreq::header::HeaderValue::from_static("zh-CN,zh;q=0.9,en;q=0.8"));
            headers.insert("authorization", wreq::header::HeaderValue::from_static("Bearer anonymous"));
            headers.insert("content-type", wreq::header::HeaderValue::from_static("application/json"));

            let mut cookie_val = if sso_token.contains(';') {
                sso_token.to_string()
            } else {
                format!("sso={}; sso-rw={}", clean_sso, clean_sso)
            };

            if let Some(ref cf) = config.cf_clearance {
                if !cookie_val.contains("cf_clearance=") {
                    cookie_val.push_str(&format!("; cf_clearance={}", cf));
                } else {
                    let re = regex::Regex::new(r"cf_clearance=[^;]*").unwrap();
                    cookie_val = re.replace(&cookie_val, format!("cf_clearance={}", cf).as_str()).to_string();
                }
            }

            if let Ok(hv) = wreq::header::HeaderValue::from_str(&cookie_val) {
                headers.insert("cookie", hv);
            }
            headers.insert("origin", wreq::header::HeaderValue::from_static("https://console.x.ai"));
            headers.insert("priority", wreq::header::HeaderValue::from_static("u=1, i"));
            headers.insert("referer", wreq::header::HeaderValue::from_static("https://console.x.ai/"));
            headers.insert("sec-fetch-dest", wreq::header::HeaderValue::from_static("empty"));
            headers.insert("sec-fetch-mode", wreq::header::HeaderValue::from_static("cors"));
            headers.insert("sec-fetch-site", wreq::header::HeaderValue::from_static("same-origin"));

            let (ua, _, _, _, _, _) = crate::chatgpt::service::generate_random_fp(
                &state.inner.read().await.impersonate_list,
                &config.user_agents_list,
            );
            if let Ok(hv) = wreq::header::HeaderValue::from_str(&ua) {
                headers.insert("user-agent", hv);
            }
            headers.insert("x-cluster", wreq::header::HeaderValue::from_static("https://us-east-1.api.x.ai"));

            info!("Sending Grok conversation request to console.x.ai, model: {}", model);
            info!("Grok request payload: {}", serde_json::to_string(&payload).unwrap_or_default());
            info!("Grok request headers: {:?}", headers);

            client.post("https://console.x.ai/v1/responses")
                .headers(headers)
                .json(&payload)
                .timeout(std::time::Duration::from_secs(120))
                .send()
                .await
        } else {
            let web_message = messages_to_web_prompt(&messages);
            let web_mode = get_web_mode_id(&model);
            let payload = json!({
                "collectionIds": [],
                "connectors": [],
                "deviceEnvInfo": {
                    "darkModeEnabled": false,
                    "devicePixelRatio": 2,
                    "screenHeight": 1329,
                    "screenWidth": 2056,
                    "viewportHeight": 1083,
                    "viewportWidth": 2056
                },
                "disableMemory": true,
                "disableSearch": false,
                "disableSelfHarmShortCircuit": false,
                "disableTextFollowUps": false,
                "enableImageGeneration": true,
                "enableImageStreaming": true,
                "enableSideBySide": true,
                "fileAttachments": [],
                "forceConcise": false,
                "forceSideBySide": false,
                "imageAttachments": [],
                "imageGenerationCount": 2,
                "isAsyncChat": false,
                "message": web_message,
                "modeId": web_mode,
                "responseMetadata": {},
                "returnImageBytes": false,
                "returnRawGrokInXaiRequest": false,
                "searchAllConnectors": false,
                "sendFinalMetadata": true,
                "temporary": true,
                "toolOverrides": {
                    "gmailSearch": false,
                    "googleCalendarSearch": false,
                    "outlookSearch": false,
                    "outlookCalendarSearch": false,
                    "googleDriveSearch": false,
                }
            });

            let mut headers = wreq::header::HeaderMap::new();
            headers.insert("accept", wreq::header::HeaderValue::from_static("*/*"));
            headers.insert("accept-encoding", wreq::header::HeaderValue::from_static("gzip, deflate, br, zstd"));
            headers.insert("accept-language", wreq::header::HeaderValue::from_static("zh-CN,zh;q=0.9,en;q=0.8"));

            // Add anti-bot detection headers (from grok2api)
            headers.insert("baggage", wreq::header::HeaderValue::from_static(
                "sentry-environment=production,sentry-release=d6add6fb0460641fd482d767a335ef72b9b6abb8,sentry-public_key=b311e0f2690c81f25e2c4cf6d4f7ce1c"
            ));

            headers.insert("content-type", wreq::header::HeaderValue::from_static("application/json"));

            // grok2api's default path only sends the two SSO cookies. Its TLS
            // impersonation and browser-like headers handle the regular Web flow.
            let cookie_val = format!("sso={}; sso-rw={}", clean_sso, clean_sso);

            if let Ok(hv) = wreq::header::HeaderValue::from_str(&cookie_val) {
                headers.insert("cookie", hv);
            }
            headers.insert("origin", wreq::header::HeaderValue::from_static("https://grok.com"));
            headers.insert("priority", wreq::header::HeaderValue::from_static("u=1, i"));
            headers.insert("referer", wreq::header::HeaderValue::from_static("https://grok.com/"));
            headers.insert("sec-fetch-dest", wreq::header::HeaderValue::from_static("empty"));
            headers.insert("sec-fetch-mode", wreq::header::HeaderValue::from_static("cors"));
            headers.insert("sec-fetch-site", wreq::header::HeaderValue::from_static("same-origin"));

            // Keep the default UA aligned with the Chrome136 TLS/HTTP2 fingerprint
            // selected by create_grok_web_client.
            let ua = if !config.user_agents_list.is_empty() {
                let (custom_ua, _, _, _, _, _) = crate::chatgpt::service::generate_random_fp(
                    &state.inner.read().await.impersonate_list,
                    &config.user_agents_list,
                );
                custom_ua
            } else {
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36".to_string()
            };
            if let Ok(hv) = wreq::header::HeaderValue::from_str(&ua) {
                headers.insert("user-agent", hv);
            }
            
            // Derive client hints from the actual User-Agent (matches Python _platform() / _major_version())
            let ua_lower = ua.to_lowercase();
            let platform = if ua_lower.contains("windows") {
                "\"Windows\""
            } else if ua_lower.contains("macintosh") || ua_lower.contains("mac os x") {
                "\"macOS\""
            } else if ua_lower.contains("android") {
                "\"Android\""
            } else if ua_lower.contains("linux") {
                "\"Linux\""
            } else {
                "\"Windows\""
            };
            let mobile = if ua_lower.contains("mobile") || ua_lower.contains("android") { "?1" } else { "?0" };
            
            // Extract Chrome major version from UA (e.g. "Chrome/136.0.0.0" → "136")
            let chrome_ver = ua.split("Chrome/")
                .nth(1)
                .and_then(|s| s.split('.').next())
                .unwrap_or("136");
            let sec_ch_ua_val = format!(
                r#""Google Chrome";v="{v}", "Chromium";v="{v}", "Not(A:Brand";v="24""#,
                v = chrome_ver
            );
            if let Ok(hv) = wreq::header::HeaderValue::from_str(&sec_ch_ua_val) {
                headers.insert("sec-ch-ua", hv);
            }
            headers.insert("sec-ch-ua-mobile", wreq::header::HeaderValue::from_str(mobile).unwrap_or_else(|_| wreq::header::HeaderValue::from_static("?0")));
            headers.insert("sec-ch-ua-model", wreq::header::HeaderValue::from_static(""));
            if let Ok(hv) = wreq::header::HeaderValue::from_str(platform) {
                headers.insert("sec-ch-ua-platform", hv);
            }
            headers.insert("upgrade-insecure-requests", wreq::header::HeaderValue::from_static("1"));
            headers.insert("sec-fetch-user", wreq::header::HeaderValue::from_static("?1"));
            
            // Python _arch() checks: aarch64/arm → "arm", x86_64/x64/win64/intel → "x86"
            // IMPORTANT: macOS Chrome UA says "Intel Mac OS X" so "intel" matches → x86
            // The "intel" check MUST come before any "mac os x" check
            let arch_val: Option<&str> = if ua_lower.contains("aarch64") || (ua_lower.contains("arm") && !ua_lower.contains("charm")) {
                Some("arm")
            } else if ua_lower.contains("x86_64") || ua_lower.contains("x64") || ua_lower.contains("win64") || ua_lower.contains("intel") {
                Some("x86")
            } else {
                None
            };

            if let Some(arch) = arch_val {
                if let Ok(hv) = wreq::header::HeaderValue::from_str(arch) {
                    headers.insert("sec-ch-ua-arch", hv);
                }
                headers.insert("sec-ch-ua-bitness", wreq::header::HeaderValue::from_static("64"));
            }
            
            // x-statsig-id: use dynamic generation matching grok2api default
            if let Ok(hv) = wreq::header::HeaderValue::from_str(&generate_statsig_id()) {
                headers.insert("x-statsig-id", hv);
            }

            // Add x-xai-request-id (UUID)
            let request_id = uuid::Uuid::new_v4().to_string();
            if let Ok(hv) = wreq::header::HeaderValue::from_str(&request_id) {
                headers.insert("x-xai-request-id", hv);
            }

            // Serialize payload manually (Python uses data=payload, not json=payload)
            let payload_bytes = serde_json::to_vec(&payload).unwrap_or_default();

            info!("Sending Grok conversation request to grok.com, model: {}, mode: {}", model, web_mode);
            info!("Grok Web request prepared (payload_bytes={})", payload_bytes.len());

            client.post("https://grok.com/rest/app-chat/conversations/new")
                .headers(headers)
                .body(payload_bytes)
                .timeout(std::time::Duration::from_secs(120))
                .send()
                .await
        };

        let response = match resp_res {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    resp
                } else {
                    let err_text = resp.text().await.unwrap_or_default();
                    error!("Grok API returned error status {}: {}", status, err_text);
                    
                    // Check if this is a Cloudflare anti-bot rejection (code:7) vs real auth error
                    let is_cloudflare_antibot = err_text.contains("\"code\":7") 
                        || err_text.contains("anti-bot")
                        || err_text.contains("Request rejected");
                    
                    if status.as_u16() == 401 || (status.as_u16() == 403 && !is_cloudflare_antibot) {
                        // Real auth failure → permanently blacklist token
                        let mut inner = state.inner.write().await;
                        if !inner.grok_error_token_list.contains(&sso_token) {
                            inner.grok_error_token_list.push(sso_token.clone());
                            let tok = sso_token.clone();
                            tokio::task::spawn_blocking(move || {
                                AppState::save_item_to_db("grok_error_tokens", &tok, &"");
                            });
                        }
                    } else if status.as_u16() == 403 && is_cloudflare_antibot {
                        // Cloudflare anti-bot 403 → only exclude from this retry cycle (NOT permanent blacklist)
                        // The token itself is likely valid; cf_clearance may be expired or fingerprint mismatch
                        warn!("Cloudflare anti-bot 403 for model {} — cf_clearance may be expired or fingerprint mismatch", model);
                        excluded_tokens.push(sso_token.clone());
                    } else if status.as_u16() == 429 {
                        // Rate limited → temporarily freeze token for 300 seconds
                        let mut inner = state.inner.write().await;
                        inner.grok_rate_limited_tokens.insert(
                            sso_token.clone(),
                            std::time::Instant::now() + std::time::Duration::from_secs(300),
                        );
                        excluded_tokens.push(sso_token.clone());
                    } else {
                        excluded_tokens.push(sso_token.clone());
                    }
                    let err = ErrorInternalServerError(format!("Grok API error (status {}): {}", status, err_text));
                    last_err = Some(err);
                    continue;
                }
            }
            Err(e) => {
                error!("Grok connection error: {:?}", e);
                excluded_tokens.push(sso_token.clone());
                let err = ErrorInternalServerError(format!("Grok connection error: {:?}", e));
                last_err = Some(err);
                continue;
            }
        };

        // Create GrokStream
        let raw_stream = Box::pin(response.bytes_stream());
        let stream_mode = if is_console {
            GrokStreamMode::Console
        } else {
            GrokStreamMode::Web
        };
        let grok_stream = GrokStream::new(raw_stream, model.clone(), tool_names.clone(), stream_mode);

        if is_stream {
            return Ok(HttpResponse::Ok()
                .content_type("text/event-stream")
                .streaming(grok_stream));
        } else {
            match format_not_stream_response(grok_stream).await {
                Ok(json_res) => return Ok(HttpResponse::Ok().json(json_res)),
                Err(e) => {
                    error!("第 {} 次尝试失败: Grok非流式聚合错误: {:?}", attempt, e);
                    last_err = Some(e);
                    continue;
                }
            }
        }
    }

    let final_err = last_err.unwrap_or_else(|| ErrorInternalServerError("Unknown Grok server error"));
    Err(final_err)
}

const TOOL_SYSTEM_HEADER: &str = "\
You have access to the following tools.

AVAILABLE TOOLS:
{tool_definitions}

TOOL CALL FORMAT — follow these rules exactly:
- When calling a tool, output ONLY the XML block below. No text before or after it.
- <parameters> must be a single-line valid JSON object (no line breaks inside).
- Place multiple tool calls inside ONE <tool_calls> element.
- Do NOT use markdown code fences around the XML.
- Do NOT output any inner monologue or explanation alongside the XML.

<tool_calls>
  <tool_call>
    <tool_name>TOOL_NAME</tool_name>
    <parameters>{{\"key\": \"value\"}}</parameters>
  </tool_call>
</tool_calls>

WRONG (never do this):
```xml
<tool_calls>...</tool_calls>
```
I'll call the search tool now. <tool_calls>...</tool_calls>

{tool_choice_instruction}
NOTE: Even if you believe you cannot fulfill the request, you must still follow the WHEN TO CALL rule above.";

const CHOICE_AUTO: &str = "WHEN TO CALL: Call a tool when it is clearly needed. Otherwise respond in plain text.";
const CHOICE_NONE: &str = "WHEN TO CALL: Do NOT call any tools. Respond in plain text only.";
const CHOICE_REQUIRED: &str = "WHEN TO CALL: You MUST output a <tool_calls> XML block. Do NOT write any plain-text reply. If you are uncertain, still call the most relevant tool with your best guess at the parameters.";
const CHOICE_FORCED: &str = "WHEN TO CALL: You MUST output a <tool_calls> XML block calling the tool named \"{name}\". Do NOT write any plain-text reply under any circumstances.";

fn format_tool_definitions(tools: &[serde_json::Value]) -> String {
    let mut parts = Vec::new();
    for tool in tools {
        if let Some(obj) = tool.as_object() {
            let func = obj.get("function").and_then(|v| v.as_object());
            if let Some(f) = func {
                let name = f.get("name").and_then(|v| v.as_str()).unwrap_or("").trim();
                let desc = f.get("description").and_then(|v| v.as_str()).unwrap_or("").trim();
                let params = f.get("parameters").cloned().unwrap_or_else(|| serde_json::json!({}));
                
                let mut lines = Vec::new();
                lines.push(format!("Tool: {}", name));
                if !desc.is_empty() {
                    lines.push(format!("Description: {}", desc));
                }
                lines.push(format!("Parameters: {}", params.to_string()));
                parts.push(lines.join("\n"));
            }
        }
    }
    parts.join("\n\n")
}

fn build_choice_instruction(tool_choice: &serde_json::Value) -> String {
    if tool_choice.is_null() {
        return CHOICE_AUTO.to_string();
    }
    if let Some(s) = tool_choice.as_str() {
        match s {
            "auto" => CHOICE_AUTO.to_string(),
            "none" => CHOICE_NONE.to_string(),
            "required" => CHOICE_REQUIRED.to_string(),
            _ => CHOICE_AUTO.to_string(),
        }
    } else if let Some(obj) = tool_choice.as_object() {
        let tc_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if tc_type == "none" {
            return CHOICE_NONE.to_string();
        }
        if tc_type == "required" {
            return CHOICE_REQUIRED.to_string();
        }
        if tc_type == "function" {
            let forced_name = obj.get("function")
                .and_then(|v| v.as_object())
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .trim();
            if !forced_name.is_empty() {
                return CHOICE_FORCED.replace("{name}", forced_name);
            }
        }
        CHOICE_AUTO.to_string()
    } else {
        CHOICE_AUTO.to_string()
    }
}

fn build_tool_system_prompt(tools: &[serde_json::Value], tool_choice: &serde_json::Value) -> String {
    let tool_defs = format_tool_definitions(tools);
    let choice_instr = build_choice_instruction(tool_choice);
    TOOL_SYSTEM_HEADER
        .replace("{tool_definitions}", &tool_defs)
        .replace("{tool_choice_instruction}", &choice_instr)
}

fn tool_calls_to_xml(tool_calls: &[serde_json::Value]) -> String {
    let mut lines = vec!["<tool_calls>".to_string()];
    for tc in tool_calls {
        if let Some(obj) = tc.as_object() {
            let func = obj.get("function").and_then(|v| v.as_object());
            if let Some(f) = func {
                let name = f.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let args = f.get("arguments").and_then(|v| v.as_str()).unwrap_or("{}");
                let norm_args = if let Ok(val) = serde_json::from_str::<serde_json::Value>(args) {
                    val.to_string()
                } else {
                    args.to_string()
                };
                lines.push("  <tool_call>".to_string());
                lines.push(format!("    <tool_name>{}</tool_name>", name));
                lines.push(format!("    <parameters>{}</parameters>", norm_args));
                lines.push("  </tool_call>".to_string());
            }
        }
    }
    lines.push("</tool_calls>".to_string());
    lines.join("\n")
}

/// Generate a randomised x-statsig-id value matching the Python grok2api _statsig_id() logic.
fn generate_statsig_id() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let msg: String = if rng.gen_bool(0.5) {
        // variant A: random 5-char alphanumeric child index
        let rand5: String = (0..5)
            .map(|_| {
                let idx = rng.gen_range(0..36u8);
                if idx < 10 { (b'0' + idx) as char } else { (b'a' + idx - 10) as char }
            })
            .collect();
        format!("x1:TypeError: Cannot read properties of null (reading 'children['{rand5}']')")
    } else {
        // variant B: random 10-char alphabetic property name
        let rand10: String = (0..10)
            .map(|_| (b'a' + rng.gen_range(0..26u8)) as char)
            .collect();
        format!("x1:TypeError: Cannot read properties of undefined (reading '{rand10}')")
    };
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(msg.as_bytes())
}
