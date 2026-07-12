use std::sync::atomic::{AtomicUsize, Ordering};
use rand::seq::SliceRandom;
use serde_json::json;
use actix_web::error::{ErrorUnauthorized, ErrorInternalServerError};
use log::{info, error};
use md5;

use crate::config::Config;
use crate::globals::AppState;
use crate::chatgpt::client::create_client;



/// 判断传入的 token 字符串是否是多个以逗号分隔的 Access Token / Refresh Token
pub fn is_multiple_tokens(req_token: &str) -> bool {
    if !req_token.contains(',') {
        return false;
    }
    let parts: Vec<&str> = req_token.split(',').collect();
    if parts.len() < 2 {
        return false;
    }
    let second = parts[1].trim();
    second.starts_with("eyJ") || second.starts_with("fk-") || second.len() > 40
}

/// 获取请求所需的真实 Token (可以是 AccessToken 或 RefreshToken)
/// state: 全局 App 状态
/// config: 全局配置文件
/// req_token: 客户端请求头传入的 Bearer Token / 授权码
/// seed_opt: 客户端可能传入的 Seed 隔离种子 (对齐 Python 官网镜像会话隔离)
pub async fn get_req_token(
    state: &AppState,
    config: &Config,
    req_token: &str,
    seed_opt: Option<&str>,
) -> Result<String, actix_web::Error> {
    let mut inner = state.inner.write().await;

    // 判断传入的 req_token 是否是多个 token 列表
    if is_multiple_tokens(req_token) {
        let parts: Vec<&str> = req_token.split(',').collect();
        let mut available_parts = Vec::new();
        for part in parts {
            let part_trimmed = part.trim().to_string();
            if !part_trimmed.is_empty() && !inner.error_token_list.contains(&part_trimmed) {
                available_parts.push(part_trimmed);
            }
        }
        if !available_parts.is_empty() {
            if config.random_token {
                let mut rng = rand::thread_rng();
                let chosen = available_parts.choose(&mut rng).unwrap().clone();
                return Ok(chosen);
            } else {
                static MULTI_TOKEN_COUNTER: AtomicUsize = AtomicUsize::new(0);
                let count = MULTI_TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed);
                let index = count % available_parts.len();
                return Ok(available_parts[index].clone());
            }
        } else {
            // 如果所有传入的 token 都在黑名单中，返回第一个 token 以便于让后续接口产生正确的未授权/报错响应
            let first_part = req_token.split(',').next().unwrap_or("").trim().to_string();
            return Ok(first_part);
        }
    }
    
    // 过滤出当前内存中可用的非错误 Token 列表
    let available_tokens: Vec<String> = inner.token_list.iter()
        .filter(|t| !inner.error_token_list.contains(t))
        .cloned()
        .collect();

    // 如果开启了自动 Seed 会话随机账号模式
    if config.auto_seed {
        if let Some(seed) = seed_opt {
            if !available_tokens.is_empty() {
                // 如果该 Seed 之前已经绑定过某后台 Token，直接复用
                if let Some(seed_val) = inner.seed_map.get(seed) {
                    if let Some(tok) = seed_val.get("token").and_then(|v| v.as_str()) {
                        return Ok(tok.to_string());
                    }
                }
                // 如果是新 Seed，随机挑选一个活跃 Token 进行一对一的强绑定
                let mut rng = rand::thread_rng();
                let chosen = available_tokens.choose(&mut rng).unwrap().clone();
                let new_seed_val = json!({
                    "token": chosen.clone(),
                    "conversations": []
                });
                inner.seed_map.insert(seed.to_string(), new_seed_val.clone());
                let sd = seed.to_string();
                let val = new_seed_val.clone();
                tokio::task::spawn_blocking(move || {
                    AppState::save_item_to_db("seed_cache", &sd, &val);
                });
                return Ok(chosen);
            }
        }

        // 验证客户端传入的是否在自定义授权码列表中。如果是，或者没有配置授权列表且传入为空，应当在后台账号池内进行轮询/随机分配
        let should_allocate = if config.authorization_list.is_empty() {
            req_token.is_empty()
        } else {
            config.authorization_list.contains(&req_token.to_string())
        };

        if should_allocate {
            if !available_tokens.is_empty() {
                if config.random_token {
                    // 随机策略：从活跃池中随机抽取
                    let mut rng = rand::thread_rng();
                    let chosen = available_tokens.choose(&mut rng).unwrap().clone();
                    return Ok(chosen);
                } else {
                    // 顺序轮询策略：使用 AtomicUsize 保证并发安全的轮询取模
                    static ROUND_ROBIN_COUNTER: AtomicUsize = AtomicUsize::new(0);
                    let count = ROUND_ROBIN_COUNTER.fetch_add(1, Ordering::Relaxed);
                    let index = count % available_tokens.len();
                    return Ok(available_tokens[index].clone());
                }
            } else {
                return Ok(String::new());
            }
        }

        Ok(req_token.to_string())
    } else {
        // 关闭自动 Seed 时，所有请求必须显式映射在 seed_map 中，否则判定为无效 Seed
        let seed = req_token;
        if let Some(seed_val) = inner.seed_map.get(seed) {
            if let Some(tok) = seed_val.get("token").and_then(|v| v.as_str()) {
                return Ok(tok.to_string());
            }
        }
        Err(ErrorUnauthorized(json!({"error": "Invalid Seed"}).to_string()))
    }
}

/// 验证并提取 AccessToken（处理 AccessToken 直通与 RefreshToken 换取 AccessToken）
pub async fn verify_token(
    state: &AppState,
    config: &Config,
    req_token: &str,
) -> Result<Option<String>, actix_web::Error> {
    // 强制验证拦截：如果配置了授权列表而传入 token 为空
    if req_token.is_empty() {
        if !config.authorization_list.is_empty() {
            error!("检测到空 Token，但配置了 Authorization 列表，拦截未授权请求。");
            return Err(ErrorUnauthorized(json!({"error": "Unauthorized"}).to_string()));
        } else {
            return Ok(None);
        }
    }

    // 格式判断
    if req_token.starts_with("eyJhbGciOi") || req_token.starts_with("fk-") {
        // 如果是标准的 JWT (AccessToken) 或者 ShareKey (fk-)，直接原样放行
        Ok(Some(req_token.to_string()))
    } else if req_token.len() == 45 {
        // 如果是 RefreshToken (长度为 45 位的 OAuth 刷新令牌)
        {
            let inner = state.inner.read().await;
            if inner.error_token_list.contains(&req_token.to_string()) {
                return Err(ErrorUnauthorized(json!({"error": "Error RefreshToken"}).to_string()));
            }
        }
        // 调用 RefreshToken 换 AccessToken 缓存/刷新方法
        let access_token = rt2ac(state, config, req_token, false).await?;
        Ok(Some(access_token))
    } else {
        // 其余情况兜底返回
        Ok(Some(req_token.to_string()))
    }
}

/// 刷新单个 RefreshToken 到 AccessToken 并写入缓存映射
/// state: 全局状态
/// config: 配置参数
/// refresh_token: 45 位 OAuth 刷新令牌
/// force_refresh: 是否强制向 OpenAI 发起远程刷新，忽略 5 天内的缓存
pub async fn rt2ac(
    state: &AppState,
    config: &Config,
    refresh_token: &str,
    force_refresh: bool,
) -> Result<String, actix_web::Error> {
    let now = chrono::Utc::now().timestamp();
    
    // 检查本地缓存是否命中，并在 5 天内直接复用缓存值
    if !force_refresh {
        let inner = state.inner.read().await;
        if let Some(val) = inner.refresh_map.get(refresh_token) {
            let timestamp = val.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0);
            let token = val.get("token").and_then(|v| v.as_str()).unwrap_or("");
            if now - timestamp < 5 * 24 * 60 * 60 && !token.is_empty() {
                return Ok(token.to_string());
            }
        }
    }

    // 缓存失效，发送 HTTP 请求至 OpenAI Auth0 服务换取新的 AccessToken
    let access_token = chat_refresh(state, config, refresh_token).await?;
    
    // 刷新成功，登记入缓存并写盘持久化
    state.update_refresh_info(
        refresh_token.to_string(),
        json!({
            "token": access_token.clone(),
            "timestamp": now
        })
    ).await;

    Ok(access_token)
}

/// 内部核心方法：调用 auth0.openai.com 刷新 access_token
async fn chat_refresh(
    state: &AppState,
    config: &Config,
    refresh_token: &str,
) -> Result<String, actix_web::Error> {
    let digest = md5::compute(refresh_token.as_bytes());
    let session_id = format!("{:x}", digest);
    
    // 为刷新客户端选用随机主代理，注入基于 Token 的 session 会话指纹以确保代理纯净
    let proxy_url = if !config.proxy_url_list.is_empty() {
        let mut rng = rand::thread_rng();
        let selected = config.proxy_url_list.choose(&mut rng).unwrap();
        Some(selected.replace("{}", &session_id))
    } else {
        None
    };

    let client = create_client(proxy_url.as_deref(), "safari15_3")
        .map_err(|e| ErrorInternalServerError(format!("Failed to create client: {:?}", e)))?;

    let payload = json!({
        "client_id": "pdlLIX2Y72MIl2rhLhTE9VV9bN905kBh", // OpenAI iOS APP 客户端的固定 ClientId
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
                        info!("使用 refresh_token 换取 OpenAI access_token 成功。");
                        return Ok(ac.to_string());
                    }
                }
                error!("解析返回的 access_token 字段失败: {}", text);
                Err(ErrorInternalServerError("Failed to parse access_token"))
            } else {
                // 如果官方响应 "invalid_grant" 或 "access_denied"，表示此 Token 已彻底过期废弃，需移入黑名单
                if text.contains("invalid_grant") || text.contains("access_denied") {
                    let mut inner = state.inner.write().await;
                    if !inner.error_token_list.contains(&refresh_token.to_string()) {
                        inner.error_token_list.push(refresh_token.to_string());
                        let tok = refresh_token.to_string();
                        tokio::task::spawn_blocking(move || {
                            AppState::save_item_to_db("error_tokens", &tok, &"");
                        });
                    }
                }
                error!("刷新 access_token 遭遇失败响应: {}", text);
                Err(ErrorInternalServerError("Failed to refresh access_token"))
            }
        }
        Err(e) => {
            error!("向 Auth0 发送刷新请求时网络发生异常: {:?}", e);
            Err(ErrorInternalServerError("Failed to send refresh request"))
        }
    }
}

/// 全局定时刷新所有后台已保存的 RefreshToken (间隔一定睡眠以防 OpenAI 触发高频风控)
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
        tokio::time::sleep(std::time::Duration::from_millis(500)).await; // 睡眠 500ms
        let _ = rt2ac(state, config, &token, force_refresh).await;
    }
    info!("已完成后台所有 RefreshToken 的自动刷新检测。");
}
