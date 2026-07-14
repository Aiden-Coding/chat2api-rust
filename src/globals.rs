use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use serde::{Serialize, Deserialize};
use log::{info, error};
use rusqlite::{Connection, params};
use crate::config::Config;

// 本地持久化保存各种运行状态和 Token 关系的常量数据文件路径
const DATA_FOLDER: &str = "data";
const TOKENS_FILE: &str = "data/token.txt";
const ERROR_TOKENS_FILE: &str = "data/error_token.txt";
const GROK_TOKENS_FILE: &str = "data/grok_token.txt";
const GROK_ERROR_TOKENS_FILE: &str = "data/grok_error_token.txt";
const DATABASE_FILE: &str = "data/data.db";

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
    pub grok_token_list: Vec<String>,        // 从 grok_token.txt 载入的全部 Grok SSO Token 列表 (含黑名单)
    pub grok_error_token_list: Vec<String>,  // 标记为异常失效的黑名单 Grok SSO Token 列表
    pub refresh_map: HashMap<String, serde_json::Value>, // 缓存：RefreshToken -> {token, timestamp} 映射
    pub wss_map: HashMap<String, WssInfo>,               // 缓存：Token -> WssInfo 映射
    pub fp_map: HashMap<String, serde_json::Value>,      // 缓存：Token -> 浏览器JA3指纹 JSON 映射
    pub seed_map: HashMap<String, serde_json::Value>,    // 缓存：Seed 随机种子 -> Token 绑定映射
    pub conversation_map: HashMap<String, serde_json::Value>, // 缓存：会话 ID 到 Token 映射
    pub impersonate_list: Vec<String>,       // 当前可用拟态指纹的名称数组 (供随机选择)
    pub limit_details: HashMap<String, HashMap<String, i64>>, // 缓存：限流频控拦截 Token -> (Model -> clears_in)
    pub grok_rate_limited_tokens: HashMap<String, std::time::Instant>, // 缓存：暂时被频控 (429) 的 Grok Token
    pub dynamic_cf_clearance: Option<String>,            // 动态由 FlareSolverr 求解获取的 clearance
    pub dynamic_user_agent: Option<String>,              // 动态由 FlareSolverr 返回的匹配 User-Agent
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

        // 3. 从 SQLite 反序列化加载各个映射表
        let conn = Self::init_db();
        let refresh_map = Self::load_map_from_db(&conn, "refresh_cache");
        let wss_map = Self::load_map_from_db(&conn, "wss_cache");
        let fp_map = Self::load_map_from_db(&conn, "fp_cache");
        let seed_map = Self::load_map_from_db(&conn, "seed_cache");
        let conversation_map = Self::load_map_from_db(&conn, "conversation_cache");

        // 1. 加载或平滑迁移主 tokens 列表
        let mut token_list = Self::load_list_from_db(&conn, "tokens");
        if token_list.is_empty() {
            // 如果 SQLite 中没有 Token，且旧 token.txt 存在，则执行迁移并存库
            if Path::new(TOKENS_FILE).exists() {
                if let Ok(content) = fs::read_to_string(TOKENS_FILE) {
                    for line in content.lines() {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() && !trimmed.starts_with('#') {
                            token_list.push(trimmed.to_string());
                            Self::save_item_to_db("tokens", trimmed, &"");
                        }
                    }
                }
            }
        }

        // 2. 加载或平滑迁移 error_tokens 列表
        let mut error_token_list = Self::load_list_from_db(&conn, "error_tokens");
        if error_token_list.is_empty() {
            // 迁移旧 error_token.txt
            if Path::new(ERROR_TOKENS_FILE).exists() {
                if let Ok(content) = fs::read_to_string(ERROR_TOKENS_FILE) {
                    for line in content.lines() {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() && !trimmed.starts_with('#') {
                            error_token_list.push(trimmed.to_string());
                            Self::save_item_to_db("error_tokens", trimmed, &"");
                        }
                    }
                }
            }
        }

        // 1.5 加载或平滑迁移主 Grok tokens 列表
        let mut grok_token_list = Self::load_list_from_db(&conn, "grok_tokens");
        if grok_token_list.is_empty() {
            if Path::new(GROK_TOKENS_FILE).exists() {
                if let Ok(content) = fs::read_to_string(GROK_TOKENS_FILE) {
                    for line in content.lines() {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() && !trimmed.starts_with('#') {
                            grok_token_list.push(trimmed.to_string());
                            Self::save_item_to_db("grok_tokens", trimmed, &"");
                        }
                    }
                }
            }
        }

        // 2.5 加载或平滑迁移 grok_error_tokens 列表
        let mut grok_error_token_list = Self::load_list_from_db(&conn, "grok_error_tokens");
        if grok_error_token_list.is_empty() {
            if Path::new(GROK_ERROR_TOKENS_FILE).exists() {
                if let Ok(content) = fs::read_to_string(GROK_ERROR_TOKENS_FILE) {
                    for line in content.lines() {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() && !trimmed.starts_with('#') {
                            grok_error_token_list.push(trimmed.to_string());
                            Self::save_item_to_db("grok_error_tokens", trimmed, &"");
                        }
                    }
                }
            }
        }

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
        if !grok_token_list.is_empty() {
            info!("加载 Grok 数据成功：主 Grok Tokens 数: {}, 故障黑名单 Grok Tokens 数: {}", grok_token_list.len(), grok_error_token_list.len());
            info!("------------------------------------------------------------");
        }

        Self {
            inner: Arc::new(RwLock::new(AppStateInner {
                token_list,
                error_token_list,
                grok_token_list,
                grok_error_token_list,
                refresh_map,
                wss_map,
                fp_map,
                seed_map,
                conversation_map,
                impersonate_list,
                limit_details: HashMap::new(),
                grok_rate_limited_tokens: HashMap::new(),
                dynamic_cf_clearance: None,
                dynamic_user_agent: None,
            })),
        }
    }

    /// 初始化 SQLite 数据库并创建缓存表
    fn init_db() -> Connection {
        let conn = Connection::open(DATABASE_FILE).expect("Failed to open SQLite database");
        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS refresh_cache (key TEXT PRIMARY KEY, val TEXT)",
            [],
        );
        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS wss_cache (key TEXT PRIMARY KEY, val TEXT)",
            [],
        );
        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS fp_cache (key TEXT PRIMARY KEY, val TEXT)",
            [],
        );
        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS seed_cache (key TEXT PRIMARY KEY, val TEXT)",
            [],
        );
        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS conversation_cache (key TEXT PRIMARY KEY, val TEXT)",
            [],
        );
        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS tokens (key TEXT PRIMARY KEY, val TEXT)",
            [],
        );
        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS error_tokens (key TEXT PRIMARY KEY, val TEXT)",
            [],
        );
        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS grok_tokens (key TEXT PRIMARY KEY, val TEXT)",
            [],
        );
        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS grok_error_tokens (key TEXT PRIMARY KEY, val TEXT)",
            [],
        );
        conn
    }

    /// 从指定的 SQLite 缓存表中加载数据到 HashMap 中
    fn load_map_from_db<V: for<'de> serde::Deserialize<'de>>(
        conn: &Connection,
        table_name: &str,
    ) -> HashMap<String, V> {
        let mut map = HashMap::new();
        let query = format!("SELECT key, val FROM {}", table_name);
        if let Ok(mut stmt) = conn.prepare(&query) {
            let rows = stmt.query_map([], |row| {
                let key: String = row.get(0)?;
                let val_str: String = row.get(1)?;
                Ok((key, val_str))
            });
            if let Ok(rows) = rows {
                for row in rows {
                    if let Ok((key, val_str)) = row {
                        if let Ok(val) = serde_json::from_str(&val_str) {
                            map.insert(key, val);
                        }
                    }
                }
            }
        }
        map
    }

    /// 从指定的 SQLite 缓存表中只读取 key 列作为字符串向量返回
    fn load_list_from_db(
        conn: &Connection,
        table_name: &str,
    ) -> Vec<String> {
        let mut list = Vec::new();
        let query = format!("SELECT key FROM {}", table_name);
        if let Ok(mut stmt) = conn.prepare(&query) {
            let rows = stmt.query_map([], |row| {
                let key: String = row.get(0)?;
                Ok(key)
            });
            if let Ok(rows) = rows {
                for row in rows {
                    if let Ok(key) = row {
                        list.push(key);
                    }
                }
            }
        }
        list
    }

    /// 公共静态方法：同步写入单条数据到指定的 SQLite 表中
    pub fn save_item_to_db<V: serde::Serialize>(table_name: &str, key: &str, val: &V) {
        if let Ok(conn) = Connection::open(DATABASE_FILE) {
            if let Ok(val_str) = serde_json::to_string(val) {
                let query = format!("INSERT OR REPLACE INTO {} (key, val) VALUES (?1, ?2)", table_name);
                if let Err(e) = conn.execute(&query, params![key, val_str]) {
                    error!("写入 SQLite 表 {} 失败: {:?}", table_name, e);
                }
            }
        } else {
            error!("无法打开 SQLite 数据库以写入表 {}", table_name);
        }
    }

    /// 公共静态方法：清空指定的 SQLite 表
    pub fn clear_table_in_db(table_name: &str) {
        if let Ok(conn) = Connection::open(DATABASE_FILE) {
            let query = format!("DELETE FROM {}", table_name);
            if let Err(e) = conn.execute(&query, []) {
                error!("清空 SQLite 表 {} 失败: {:?}", table_name, e);
            }
        } else {
            error!("无法打开 SQLite 数据库以清空表 {}", table_name);
        }
    }

    /// 保存当前的 Token 列表并同步写盘
    pub async fn save_token_list(&self, tokens: Vec<String>) {
        {
            let mut inner = self.inner.write().await;
            inner.token_list = tokens.clone();
        }
        tokio::task::spawn_blocking(move || {
            Self::clear_table_in_db("tokens");
            for token in tokens {
                Self::save_item_to_db("tokens", &token, &"");
            }
        }).await.unwrap_or_else(|e| error!("spawn_blocking 写入 tokens 失败: {:?}", e));
    }

    /// 追加单个 Token 到内存和本地文件中（去重处理）
    pub async fn append_token(&self, token: &str) -> bool {
        let trimmed = token.trim().to_string();
        if !trimmed.is_empty() && !trimmed.starts_with('#') {
            {
                let mut inner = self.inner.write().await;
                if inner.token_list.contains(&trimmed) {
                    return false; // 重复，跳过
                }
                inner.token_list.push(trimmed.clone());
            }
            let tok = trimmed;
            tokio::task::spawn_blocking(move || {
                Self::save_item_to_db("tokens", &tok, &"");
            }).await.unwrap_or_else(|e| error!("spawn_blocking 追加 token 失败: {:?}", e));
            return true;
        }
        false
    }

    /// 清空所有内存的 Token 和 error Token，并清空对应文件
    pub async fn clear_tokens(&self) {
        {
            let mut inner = self.inner.write().await;
            inner.token_list.clear();
            inner.error_token_list.clear();
        }
        tokio::task::spawn_blocking(|| {
            Self::clear_table_in_db("tokens");
            Self::clear_table_in_db("error_tokens");
        }).await.unwrap_or_else(|e| error!("spawn_blocking 清空 tokens 失败: {:?}", e));
    }

    /// 从内存和数据库中批量删除指定的 Token 凭证
    pub async fn delete_tokens(&self, tokens_to_delete: Vec<String>) {
        {
            let mut inner = self.inner.write().await;
            for token in &tokens_to_delete {
                inner.token_list.retain(|t| t != token);
                inner.error_token_list.retain(|t| t != token);
            }
        }
        tokio::task::spawn_blocking(move || {
            if let Ok(conn) = Connection::open(DATABASE_FILE) {
                for token in tokens_to_delete {
                    let _ = conn.execute("DELETE FROM tokens WHERE key = ?1", params![token]);
                    let _ = conn.execute("DELETE FROM error_tokens WHERE key = ?1", params![token]);
                }
            } else {
                error!("无法打开 SQLite 数据库以执行删除 token");
            }
        }).await.unwrap_or_else(|e| error!("spawn_blocking 删除 tokens 失败: {:?}", e));
    }

    /// 保存当前的 Grok Token 列表并同步写盘
    pub async fn save_grok_token_list(&self, tokens: Vec<String>) {
        {
            let mut inner = self.inner.write().await;
            inner.grok_token_list = tokens.clone();
        }
        tokio::task::spawn_blocking(move || {
            Self::clear_table_in_db("grok_tokens");
            for token in tokens {
                Self::save_item_to_db("grok_tokens", &token, &"");
            }
        }).await.unwrap_or_else(|e| error!("spawn_blocking 写入 grok_tokens 失败: {:?}", e));
    }

    /// 追加单个 Grok Token 到内存和本地文件中（去重处理）
    pub async fn append_grok_token(&self, token: &str) -> bool {
        let trimmed = token.trim().to_string();
        if !trimmed.is_empty() && !trimmed.starts_with('#') {
            {
                let mut inner = self.inner.write().await;
                if inner.grok_token_list.contains(&trimmed) {
                    return false; // 重复，跳过
                }
                inner.grok_token_list.push(trimmed.clone());
            }
            let tok = trimmed;
            tokio::task::spawn_blocking(move || {
                Self::save_item_to_db("grok_tokens", &tok, &"");
            }).await.unwrap_or_else(|e| error!("spawn_blocking 追加 grok token 失败: {:?}", e));
            return true;
        }
        false
    }

    /// 清空所有内存的 Grok Token 和 error Grok Token，并清空对应文件
    pub async fn clear_grok_tokens(&self) {
        {
            let mut inner = self.inner.write().await;
            inner.grok_token_list.clear();
            inner.grok_error_token_list.clear();
        }
        tokio::task::spawn_blocking(|| {
            Self::clear_table_in_db("grok_tokens");
            Self::clear_table_in_db("grok_error_tokens");
        }).await.unwrap_or_else(|e| error!("spawn_blocking 清空 grok tokens 失败: {:?}", e));
    }

    /// 从内存和数据库中批量删除指定的 Grok Token 凭证
    pub async fn delete_grok_tokens(&self, tokens_to_delete: Vec<String>) {
        {
            let mut inner = self.inner.write().await;
            for token in &tokens_to_delete {
                inner.grok_token_list.retain(|t| t != token);
                inner.grok_error_token_list.retain(|t| t != token);
            }
        }
        tokio::task::spawn_blocking(move || {
            if let Ok(conn) = Connection::open(DATABASE_FILE) {
                for token in tokens_to_delete {
                    let _ = conn.execute("DELETE FROM grok_tokens WHERE key = ?1", params![token]);
                    let _ = conn.execute("DELETE FROM grok_error_tokens WHERE key = ?1", params![token]);
                }
            } else {
                error!("无法打开 SQLite 数据库以执行删除 grok token");
            }
        }).await.unwrap_or_else(|e| error!("spawn_blocking 删除 grok tokens 失败: {:?}", e));
    }

    /// 更新并写盘 RefreshToken 映射数据
    pub async fn update_refresh_info(&self, token: String, info: serde_json::Value) {
        {
            let mut inner = self.inner.write().await;
            inner.refresh_map.insert(token.clone(), info.clone());
        }
        let tok = token;
        let inf = info;
        tokio::task::spawn_blocking(move || {
            Self::save_item_to_db("refresh_cache", &tok, &inf);
        }).await.unwrap_or_else(|e| error!("spawn_blocking 写入 refresh_cache 失败: {:?}", e));
    }

    /// 插入并保存最新的 Wss 接入状态
    pub async fn update_wss_info(&self, token: String, wss_mode: bool, wss_url: Option<String>) {
        let now = chrono::Utc::now().timestamp();
        let wss_info = WssInfo {
            timestamp: now,
            wss_url,
            wss_mode,
        };
        {
            let mut inner = self.inner.write().await;
            inner.wss_map.insert(token.clone(), wss_info.clone());
        }
        let tok = token;
        tokio::task::spawn_blocking(move || {
            Self::save_item_to_db("wss_cache", &tok, &wss_info);
        }).await.unwrap_or_else(|e| error!("spawn_blocking 写入 wss_cache 失败: {:?}", e));
    }

    /// 清空会话隔离关系的种子映射，并同步回写至磁盘上
    pub async fn clear_seed_tokens(&self) {
        {
            let mut inner = self.inner.write().await;
            inner.seed_map.clear();
            inner.conversation_map.clear();
        }
        tokio::task::spawn_blocking(|| {
            Self::clear_table_in_db("seed_cache");
            Self::clear_table_in_db("conversation_cache");
        }).await.unwrap_or_else(|e| error!("spawn_blocking 清空 seed 和 conversation 缓存失败: {:?}", e));
    }

    /// 刷新并写盘当前会话与 Token 的映射关系 (已废弃，保留空实现以兼容)
    pub async fn save_conversation_map(&self) {}

    /// 更新会话和 Token 的映射关系
    pub async fn update_conversation_info(&self, conversation_id: String, token: serde_json::Value) {
        {
            let mut inner = self.inner.write().await;
            inner.conversation_map.insert(conversation_id.clone(), token.clone());
        }
        let cid = conversation_id;
        let tok = token;
        tokio::task::spawn_blocking(move || {
            Self::save_item_to_db("conversation_cache", &cid, &tok);
        }).await.unwrap_or_else(|e| error!("spawn_blocking 写入 conversation_cache 失败: {:?}", e));
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
