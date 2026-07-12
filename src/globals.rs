use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use serde::{Serialize, Deserialize};
use log::{info, error};
use crate::config::Config;

const DATA_FOLDER: &str = "data";
const TOKENS_FILE: &str = "data/token.txt";
const REFRESH_MAP_FILE: &str = "data/refresh_map.json";
const ERROR_TOKENS_FILE: &str = "data/error_token.txt";
const WSS_MAP_FILE: &str = "data/wss_map.json";
const FP_FILE: &str = "data/fp_map.json";
const SEED_MAP_FILE: &str = "data/seed_map.json";
const CONVERSATION_MAP_FILE: &str = "data/conversation_map.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WssInfo {
    pub timestamp: i64,
    pub wss_url: Option<String>,
    pub wss_mode: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshInfo {
    pub access_token: String,
    pub expires_at: i64,
}

#[derive(Debug)]
pub struct AppStateInner {
    pub token_list: Vec<String>,
    pub error_token_list: Vec<String>,
    pub refresh_map: HashMap<String, serde_json::Value>, // Token -> RefreshInfo 或者 动态 JSON
    pub wss_map: HashMap<String, WssInfo>,               // Token -> WssInfo
    pub fp_map: HashMap<String, serde_json::Value>,      // Token -> 浏览器指纹 JSON
    pub seed_map: HashMap<String, serde_json::Value>,
    pub conversation_map: HashMap<String, serde_json::Value>,
    pub impersonate_list: Vec<String>,
    pub limit_details: HashMap<String, HashMap<String, i64>>, // Token -> (Model -> clears_in_timestamp)
}

#[derive(Clone)]
pub struct AppState {
    pub inner: Arc<RwLock<AppStateInner>>,
}

impl AppState {
    pub fn new(config: &Config) -> Self {
        // 创建 data 文件夹
        if !Path::new(DATA_FOLDER).exists() {
            let _ = fs::create_dir_all(DATA_FOLDER);
        }

        let mut token_list = Vec::new();
        if Path::new(TOKENS_FILE).exists() {
            if let Ok(content) = fs::read_to_string(TOKENS_FILE) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() && !trimmed.starts_with('#') {
                        token_list.push(trimmed.to_string());
                    }
                }
            }
        } else {
            let _ = fs::write(TOKENS_FILE, "");
        }

        let mut error_token_list = Vec::new();
        if Path::new(ERROR_TOKENS_FILE).exists() {
            if let Ok(content) = fs::read_to_string(ERROR_TOKENS_FILE) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() && !trimmed.starts_with('#') {
                        error_token_list.push(trimmed.to_string());
                    }
                }
            }
        } else {
            let _ = fs::write(ERROR_TOKENS_FILE, "");
        }

        let refresh_map = Self::load_json_map(REFRESH_MAP_FILE);
        let wss_map = Self::load_json_map(WSS_MAP_FILE);
        let fp_map = Self::load_json_map(FP_FILE);
        let seed_map = Self::load_json_map(SEED_MAP_FILE);
        let conversation_map = Self::load_json_map(CONVERSATION_MAP_FILE);

        let impersonate_list = if config.impersonate_list.is_empty() {
            vec![
                "chrome99".to_string(),
                "chrome100".to_string(),
                "chrome101".to_string(),
                "chrome104".to_string(),
                "chrome107".to_string(),
                "chrome110".to_string(),
                "chrome116".to_string(),
                "chrome119".to_string(),
                "chrome120".to_string(),
                "chrome123".to_string(),
                "edge99".to_string(),
                "edge101".to_string(),
            ]
        } else {
            config.impersonate_list.clone()
        };

        if !token_list.is_empty() {
            info!("Token list count: {}, Error token list count: {}", token_list.len(), error_token_list.len());
            info!("------------------------------------------------------------");
        }

        Self {
            inner: Arc::new(RwLock::new(AppStateInner {
                token_list,
                error_token_list,
                refresh_map,
                wss_map,
                fp_map,
                seed_map,
                conversation_map,
                impersonate_list,
                limit_details: HashMap::new(),
            })),
        }
    }

    fn load_json_map<V: for<'de> Deserialize<'de>>(file_path: &str) -> HashMap<String, V> {
        if Path::new(file_path).exists() {
            if let Ok(content) = fs::read_to_string(file_path) {
                if let Ok(map) = serde_json::from_str(&content) {
                    return map;
                }
            }
        }
        HashMap::new()
    }

    fn save_json_map<V: Serialize>(file_path: &str, map: &HashMap<String, V>) {
        if let Ok(content) = serde_json::to_string_pretty(map) {
            if let Err(e) = fs::write(file_path, content) {
                error!("Failed to write to file {}: {:?}", file_path, e);
            }
        }
    }

    // 各种状态保存辅助方法
    pub async fn save_token_list(&self, tokens: Vec<String>) {
        let mut inner = self.inner.write().await;
        inner.token_list = tokens;
        let content = inner.token_list.join("\n");
        if let Err(e) = fs::write(TOKENS_FILE, content) {
            error!("Failed to save token list to file: {:?}", e);
        }
    }

    pub async fn append_token(&self, token: &str) {
        let mut inner = self.inner.write().await;
        let trimmed = token.trim().to_string();
        if !trimmed.is_empty() && !trimmed.starts_with('#') {
            inner.token_list.push(trimmed.clone());
            // 追加写入文件
            if let Ok(mut content) = fs::read_to_string(TOKENS_FILE) {
                if !content.ends_with('\n') && !content.is_empty() {
                    content.push('\n');
                }
                content.push_str(&trimmed);
                content.push('\n');
                let _ = fs::write(TOKENS_FILE, content);
            } else {
                let _ = fs::write(TOKENS_FILE, format!("{}\n", trimmed));
            }
        }
    }

    pub async fn clear_tokens(&self) {
        let mut inner = self.inner.write().await;
        inner.token_list.clear();
        inner.error_token_list.clear();
        let _ = fs::write(TOKENS_FILE, "");
        let _ = fs::write(ERROR_TOKENS_FILE, "");
    }

    pub async fn save_refresh_map(&self) {
        let inner = self.inner.read().await;
        Self::save_json_map(REFRESH_MAP_FILE, &inner.refresh_map);
    }

    pub async fn update_refresh_info(&self, token: String, info: serde_json::Value) {
        {
            let mut inner = self.inner.write().await;
            inner.refresh_map.insert(token, info);
        }
        self.save_refresh_map().await;
    }

    pub async fn save_wss_map(&self) {
        let inner = self.inner.read().await;
        Self::save_json_map(WSS_MAP_FILE, &inner.wss_map);
    }

    pub async fn update_wss_info(&self, token: String, wss_mode: bool, wss_url: Option<String>) {
        {
            let mut inner = self.inner.write().await;
            let now = chrono::Utc::now().timestamp();
            inner.wss_map.insert(token, WssInfo {
                timestamp: now,
                wss_url,
                wss_mode,
            });
        }
        self.save_wss_map().await;
    }

    pub async fn save_fp_map(&self) {
        let inner = self.inner.read().await;
        Self::save_json_map(FP_FILE, &inner.fp_map);
    }

    pub async fn save_seed_map(&self) {
        let inner = self.inner.read().await;
        Self::save_json_map(SEED_MAP_FILE, &inner.seed_map);
    }

    pub async fn clear_seed_tokens(&self) {
        let mut inner = self.inner.write().await;
        inner.seed_map.clear();
        inner.conversation_map.clear();
        Self::save_json_map(SEED_MAP_FILE, &inner.seed_map);
        Self::save_json_map(CONVERSATION_MAP_FILE, &inner.conversation_map);
    }

    pub async fn save_conversation_map(&self) {
        let inner = self.inner.read().await;
        Self::save_json_map(CONVERSATION_MAP_FILE, &inner.conversation_map);
    }

    // 本地频控记录
    pub async fn check_is_limit(&self, token: &str, model: &str, clears_in: i64) {
        if token.is_empty() {
            return;
        }
        let now = chrono::Utc::now().timestamp();
        let clear_time = now + clears_in;
        let mut inner = self.inner.write().await;
        inner.limit_details
            .entry(token.to_string())
            .or_insert_with(HashMap::new)
            .insert(model.to_string(), clear_time);
        
        let local_dt = chrono::DateTime::<chrono::Utc>::from_timestamp(clear_time, 0)
            .map(|dt| dt.with_timezone(&chrono::Local))
            .unwrap_or_else(|| chrono::Local::now());
        info!("{}: Reached {} limit, will be cleared at {}", 
            if token.len() > 40 { &token[..40] } else { token },
            model,
            local_dt.format("%Y-%m-%d %H:%M:%S")
        );
    }

    // 本地频控拦截检查
    pub async fn handle_request_limit(&self, token: &str, model: &str) -> Option<String> {
        if token.is_empty() {
            return None;
        }
        let mut inner = self.inner.write().await;
        if let Some(models_map) = inner.limit_details.get_mut(token) {
            if let Some(&limit_time) = models_map.get(model) {
                let now = chrono::Utc::now().timestamp();
                if limit_time > now {
                    let local_dt = chrono::DateTime::<chrono::Utc>::from_timestamp(limit_time, 0)
                        .map(|dt| dt.with_timezone(&chrono::Local))
                        .unwrap_or_else(|| chrono::Local::now());
                    let msg = format!(
                        "Request limit exceeded. You can continue with the default model now, or try again after {}",
                        local_dt.format("%Y-%m-%d %H:%M:%S")
                    );
                    info!("{}", msg);
                    return Some(msg);
                } else {
                    models_map.remove(model);
                }
            }
        }
        None
    }
}
