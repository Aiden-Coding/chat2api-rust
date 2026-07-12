use std::fs;
use rand::seq::SliceRandom;
use serde_json::json;
use actix_web::error::{ErrorUnauthorized, ErrorInternalServerError};
use log::{info, error};
use md5;

use crate::config::Config;
use crate::globals::{AppState, RefreshInfo};
use crate::chatgpt::client::create_client;

const ERROR_TOKENS_FILE: &str = "data/error_token.txt";
const SEED_MAP_FILE: &str = "data/seed_map.json";

// 获取请求所需的真实 Token (AccessToken 或 RefreshToken)
pub async fn get_req_token(
    state: &AppState,
    config: &Config,
    req_token: &str,
    seed_opt: Option<&str>,
) -> Result<String, actix_web::Error> {
    let mut inner = state.inner.write().await;
    
    // 过滤出可用 Token 列表
    let available_tokens: Vec<String> = inner.token_list.iter()
        .filter(|t| !inner.error_token_list.contains(t))
        .cloned()
        .collect();

    if config.auto_seed {
        if let Some(seed) = seed_opt {
            if !available_tokens.is_empty() {
                if let Some(seed_val) = inner.seed_map.get(seed) {
                    if let Some(tok) = seed_val.get("token").and_then(|v| v.as_str()) {
                        return Ok(tok.to_string());
                    }
                }
                // 如果不存在，随机绑定一个
                let mut rng = rand::thread_rng();
                let chosen = available_tokens.choose(&mut rng).unwrap().clone();
                let new_seed_val = json!({
                    "token": chosen.clone(),
                    "conversations": []
                });
                inner.seed_map.insert(seed.to_string(), new_seed_val);
                
                // 持久化 seed_map
                if let Ok(content) = serde_json::to_string_pretty(&inner.seed_map) {
                    let _ = fs::write(SEED_MAP_FILE, content);
                }
                return Ok(chosen);
            }
        }

        // 验证 req_token 是否在本地授权码列表里，如果是，则说明是授权请求，应当轮询后台 Token
        if config.authorization_list.contains(&req_token.to_string()) {
            if !available_tokens.is_empty() {
                if config.random_token {
                    let mut rng = rand::thread_rng();
                    let chosen = available_tokens.choose(&mut rng).unwrap().clone();
                    return Ok(chosen);
                } else {
                    // 顺序轮询，在全局维护一个计数器
                    // 我们可以在 globals.rs 里面用全局 AtomicUsize 或简单加锁更新。
                    // 简单起见，从 inner 里每次自增
                    static mut ROUND_ROBIN_COUNTER: usize = 0;
                    unsafe {
                        ROUND_ROBIN_COUNTER += 1;
                        let index = ROUND_ROBIN_COUNTER % available_tokens.len();
                        return Ok(available_tokens[index].clone());
                    }
                }
            } else {
                return Ok(String::new());
            }
        }

        Ok(req_token.to_string())
    } else {
        // 关闭自动 Seed (随机账号匹配) 时，所有请求必须基于已有的 Seed
        let seed = req_token;
        if let Some(seed_val) = inner.seed_map.get(seed) {
            if let Some(tok) = seed_val.get("token").and_then(|v| v.as_str()) {
                return Ok(tok.to_string());
            }
        }
        Err(ErrorUnauthorized(json!({"error": "Invalid Seed"}).to_string()))
    }
}

// 验证并提取 AccessToken
pub async fn verify_token(
    state: &AppState,
    config: &Config,
    req_token: &str,
) -> Result<Option<String>, actix_web::Error> {
    if req_token.is_empty() {
        if !config.authorization_list.is_empty() {
            error!("Unauthorized with empty token.");
            return Err(ErrorUnauthorized(json!({"error": "Unauthorized"}).to_string()));
        } else {
            return Ok(None);
        }
    }

    if req_token.starts_with("eyJhbGciOi") || req_token.starts_with("fk-") {
        // 直接是 AccessToken 或者以 fk- 开头的 Key，直接返回
        Ok(Some(req_token.to_string()))
    } else if req_token.len() == 45 {
        // RefreshToken，需要获取或刷新 AccessToken
        {
            let inner = state.inner.read().await;
            if inner.error_token_list.contains(&req_token.to_string()) {
                return Err(ErrorUnauthorized(json!({"error": "Error RefreshToken"}).to_string()));
            }
        }

        let access_token = rt2ac(state, config, req_token, false).await?;
        Ok(Some(access_token))
    } else {
        // 兜底返回原 Token
        Ok(Some(req_token.to_string()))
    }
}

// 刷新单个 RefreshToken 到 AccessToken
pub async fn rt2ac(
    state: &AppState,
    config: &Config,
    refresh_token: &str,
    force_refresh: bool,
) -> Result<String, actix_web::Error> {
    let now = chrono::Utc::now().timestamp();
    
    // 检查缓存
    if !force_refresh {
        let inner = state.inner.read().await;
        if let Some(val) = inner.refresh_map.get(refresh_token) {
            let timestamp = val.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0);
            let token = val.get("token").and_then(|v| v.as_str()).unwrap_or("");
            // 缓存有效时间为 5 天
            if now - timestamp < 5 * 24 * 60 * 60 && !token.is_empty() {
                return Ok(token.to_string());
            }
        }
    }

    // 缓存失效，发送 HTTP 请求刷新
    let access_token = chat_refresh(state, config, refresh_token).await?;
    
    // 更新缓存并保存
    state.update_refresh_info(
        refresh_token.to_string(),
        json!({
            "token": access_token.clone(),
            "timestamp": now
        })
    ).await;

    Ok(access_token)
}

// 内部调用 auth0 接口刷新 access_token
async fn chat_refresh(
    state: &AppState,
    config: &Config,
    refresh_token: &str,
) -> Result<String, actix_web::Error> {
    let digest = md5::compute(refresh_token.as_bytes());
    let session_id = format!("{:x}", digest);
    
    let proxy_url = if !config.proxy_url_list.is_empty() {
        let mut rng = rand::thread_rng();
        let selected = config.proxy_url_list.choose(&mut rng).unwrap();
        Some(selected.replace("{}", &session_id))
    } else {
        None
    };

    let client = create_client(proxy_url.as_deref())
        .map_err(|e| ErrorInternalServerError(format!("Failed to create client: {:?}", e)))?;

    let payload = json!({
        "client_id": "pdlLIX2Y72MIl2rhLhTE9VV9bN905kBh",
        "grant_type": "refresh_token",
        "redirect_uri": "com.openai.chat://auth0.openai.com/ios/com.openai.chat/callback",
        "refresh_token": refresh_token
    });

    let resp_res = client.post("https://auth0.openai.com/oauth/token")
        .json(&payload)
        .send()
        .await;

    match resp_res {
        Ok(resp) => {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if status.is_success() {
                if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let Some(ac) = json_val.get("access_token").and_then(|v| v.as_str()) {
                        info!("refresh_token -> access_token with openai success.");
                        return Ok(ac.to_string());
                    }
                }
                error!("Failed to parse access_token from response: {}", text);
                Err(ErrorInternalServerError("Failed to parse access_token"))
            } else {
                // 如果是无效 token，加入错误列表
                if text.contains("invalid_grant") || text.contains("access_denied") {
                    let mut inner = state.inner.write().await;
                    if !inner.error_token_list.contains(&refresh_token.to_string()) {
                        inner.error_token_list.push(refresh_token.to_string());
                        
                        // 追加写入到错误 Token 文件
                        if let Ok(mut content) = fs::read_to_string(ERROR_TOKENS_FILE) {
                            if !content.ends_with('\n') && !content.is_empty() {
                                content.push('\n');
                            }
                            content.push_str(refresh_token);
                            content.push('\n');
                            let _ = fs::write(ERROR_TOKENS_FILE, content);
                        } else {
                            let _ = fs::write(ERROR_TOKENS_FILE, format!("{}\n", refresh_token));
                        }
                    }
                }
                error!("Failed to refresh access_token, response: {}", text);
                Err(ErrorInternalServerError("Failed to refresh access_token"))
            }
        }
        Err(e) => {
            error!("Failed to send refresh request: {:?}", e);
            Err(ErrorInternalServerError("Failed to send refresh request"))
        }
    }
}

// 批量刷新所有 Token
pub async fn refresh_all_tokens(
    state: &AppState,
    config: &Config,
    force_refresh: bool,
) {
    let tokens: Vec<String> = {
        let inner = state.inner.read().await;
        inner.token_list.iter()
            .filter(|t| !inner.error_token_list.contains(t) && t.len() == 45)
            .cloned()
            .collect()
    };

    for token in tokens {
        // 睡眠一段时间防风控
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let _ = rt2ac(state, config, &token, force_refresh).await;
    }
    info!("All tokens refresh completed.");
}
