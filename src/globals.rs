use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use serde::{Serialize, Deserialize};
use log::{info, error};
use crate::config::Config;

// 本地持久化保存各种运行状态和 Token 关系的常量数据文件路径
const DATA_FOLDER: &str = "data";
const TOKENS_FILE: &str = "data/token.txt";
const REFRESH_MAP_FILE: &str = "data/refresh_map.json";
const ERROR_TOKENS_FILE: &str = "data/error_token.txt";
const WSS_MAP_FILE: &str = "data/wss_map.json";
const FP_FILE: &str = "data/fp_map.json";
const SEED_MAP_FILE: &str = "data/seed_map.json";
const CONVERSATION_MAP_FILE: &str = "data/conversation_map.json";

/// 记录 WebSocket 握手状态与 URL 映射的元数据结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WssInfo {
    pub timestamp: i64,          // 记录生成时的时间戳
    pub wss_url: Option<String>, // 返回的 ws 连接地址
    pub wss_mode: bool,          // 该 Token 是否处于 WebSocket 接入模式
}

/// 记录 RefreshToken 缓存映射元数据结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshInfo {
    pub access_token: String,    // 兑换出的 AccessToken
    pub expires_at: i64,         // 缓存过期 Unix 时间戳
}

/// 程序内存运行中的读写锁保护状态结构 (被 AppState 共享引用)
#[derive(Debug)]
pub struct AppStateInner {
    pub token_list: Vec<String>,             // 从 token.txt 载入的全部 Token 列表 (含黑名单)
    pub error_token_list: Vec<String>,       // 标记为异常失效的黑名单 Token 列表
    pub refresh_map: HashMap<String, serde_json::Value>, // 缓存：RefreshToken -> {token, timestamp} 映射
    pub wss_map: HashMap<String, WssInfo>,               // 缓存：Token -> WssInfo 映射
    pub fp_map: HashMap<String, serde_json::Value>,      // 缓存：Token -> 浏览器JA3指纹 JSON 映射
    pub seed_map: HashMap<String, serde_json::Value>,    // 缓存：Seed 随机种子 -> Token 绑定映射
    pub conversation_map: HashMap<String, serde_json::Value>, // 缓存：会话 ID 到 Token 映射
    pub impersonate_list: Vec<String>,       // 当前可用拟态指纹的名称数组 (供随机选择)
    pub limit_details: HashMap<String, HashMap<String, i64>>, // 缓存：限流频控拦截 Token -> (Model -> clears_in)
}

/// 全局共享状态的包装结构体，采用多线程安全的 Arc + 异步读写锁 RwLock 维护
#[derive(Clone)]
pub struct AppState {
    pub inner: Arc<RwLock<AppStateInner>>,
}

impl AppState {
    /// 构造函数：初始化并从本地文件系统中反序列化恢复各缓存列表
    pub fn new(config: &Config) -> Self {
        // 如果 data 文件夹不存在则创建它
        if !Path::new(DATA_FOLDER).exists() {
            let _ = fs::create_dir_all(DATA_FOLDER);
        }

        // 1. 加载主 tokens.txt 文本列表
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

        // 2. 加载 error_token.txt 异常账号列表
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

        // 3. 反序列化加载各个 JSON 结构体映射表
        let refresh_map = Self::load_json_map(REFRESH_MAP_FILE);
        let wss_map = Self::load_json_map(WSS_MAP_FILE);
        let fp_map = Self::load_json_map(FP_FILE);
        let seed_map = Self::load_json_map(SEED_MAP_FILE);
        let conversation_map = Self::load_json_map(CONVERSATION_MAP_FILE);

        // 如果用户在环境变量中未配置混淆指纹，则装载默认的主流指纹名称列表
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
            info!("加载数据成功：主 Tokens 数: {}, 故障黑名单 Tokens 数: {}", token_list.len(), error_token_list.len());
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

    /// 辅助泛型：从本地磁盘反序列化读取 JSON 字典
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

    /// 辅助泛型：将指定的 JSON 映射字典优雅持久化写回至磁盘文件
    fn save_json_map<V: Serialize>(file_path: &str, map: &HashMap<String, V>) {
        if let Ok(content) = serde_json::to_string_pretty(map) {
            if let Err(e) = fs::write(file_path, content) {
                error!("写入 JSON 配置文件 {} 时发生磁盘故障: {:?}", file_path, e);
            }
        }
    }

    /// 保存当前的 Token 列表并同步写盘
    pub async fn save_token_list(&self, tokens: Vec<String>) {
        let mut inner = self.inner.write().await;
        inner.token_list = tokens;
        let content = inner.token_list.join("\n");
        if let Err(e) = fs::write(TOKENS_FILE, content) {
            error!("无法保存 Token 列表到本地磁盘: {:?}", e);
        }
    }

    /// 追加单个 Token 到内存和本地文件中
    pub async fn append_token(&self, token: &str) {
        let mut inner = self.inner.write().await;
        let trimmed = token.trim().to_string();
        if !trimmed.is_empty() && !trimmed.starts_with('#') {
            inner.token_list.push(trimmed.clone());
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

    /// 清空所有内存的 Token 和 error Token，并清空对应文件
    pub async fn clear_tokens(&self) {
        let mut inner = self.inner.write().await;
        inner.token_list.clear();
        inner.error_token_list.clear();
        let _ = fs::write(TOKENS_FILE, "");
        let _ = fs::write(ERROR_TOKENS_FILE, "");
    }

    /// 刷新 Refresh 缓存字典并写盘
    pub async fn save_refresh_map(&self) {
        let inner = self.inner.read().await;
        Self::save_json_map(REFRESH_MAP_FILE, &inner.refresh_map);
    }

    /// 更新并写盘 RefreshToken 映射数据
    pub async fn update_refresh_info(&self, token: String, info: serde_json::Value) {
        {
            let mut inner = self.inner.write().await;
            inner.refresh_map.insert(token, info);
        }
        self.save_refresh_map().await;
    }

    /// 刷新并写盘 Wss 映射字典
    pub async fn save_wss_map(&self) {
        let inner = self.inner.read().await;
        Self::save_json_map(WSS_MAP_FILE, &inner.wss_map);
    }

    /// 插入并保存最新的 Wss 接入状态
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

    /// 刷新并持久化写盘浏览器伪装指纹 (JA3/UserAgent)
    pub async fn save_fp_map(&self) {
        let inner = self.inner.read().await;
        Self::save_json_map(FP_FILE, &inner.fp_map);
    }

    /// 刷新并持久化写盘 Seed 映射关系
    pub async fn save_seed_map(&self) {
        let inner = self.inner.read().await;
        Self::save_json_map(SEED_MAP_FILE, &inner.seed_map);
    }

    /// 清空会话隔离关系的种子映射，并同步回写至磁盘上
    pub async fn clear_seed_tokens(&self) {
        let mut inner = self.inner.write().await;
        inner.seed_map.clear();
        inner.conversation_map.clear();
        Self::save_json_map(SEED_MAP_FILE, &inner.seed_map);
        Self::save_json_map(CONVERSATION_MAP_FILE, &inner.conversation_map);
    }

    /// 刷新并写盘当前会话与 Token 的映射关系
    pub async fn save_conversation_map(&self) {
        let inner = self.inner.read().await;
        Self::save_json_map(CONVERSATION_MAP_FILE, &inner.conversation_map);
    }

    /// 当某个 Token 触发 OpenAI 429 会话限流时，本地记录它的限制模型与释放截止时间 (UTC+Local)
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
        
        // 自动转换美东/标准时间为服务器本地运行的时区时间并显示
        let local_dt = chrono::DateTime::<chrono::Utc>::from_timestamp(clear_time, 0)
            .map(|dt| dt.with_timezone(&chrono::Local))
            .unwrap_or_else(|| chrono::Local::now());
        info!("{}: 触发模型 {} 官方频控限制，预计自动释放解除时间为: {}", 
            if token.len() > 40 { &token[..40] } else { token },
            model,
            local_dt.format("%Y-%m-%d %H:%M:%S")
        );
    }

    /// 本地拦截校验方法：在每次请求发送前判断该 Token 与该模型是否仍在频控限制期内
    /// 返回 Option<String>。如果有值，说明被频控，返回提示文案；如果是 None 则放行
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
                    info!("本地频控直接拦截: {}", msg);
                    return Some(msg);
                } else {
                    // 超过时限，说明限制已自动解除，从列表中摘除
                    models_map.remove(model);
                }
            }
        }
        None
    }
}
