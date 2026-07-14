use std::env;
use std::fs;
use log::info;

/// 本项目所有的全局环境配置项结构体
#[derive(Debug, Clone)]
pub struct Config {
    pub api_prefix: Option<String>,            // 自定义接口请求的前缀密码 (防止暴露)
    pub authorization_list: Vec<String>,       // 自定义配置的 Bearer Token 授权码列表
    pub chatgpt_base_url_list: Vec<String>,    // ChatGPT 逆向后端接口网关地址列表
    pub auth_key: Option<String>,              // 专用授权 Header 的密钥值
    pub x_sign: Option<String>,                // 自定义请求签名头信息
    pub ark0se_token_url_list: Vec<String>,    // 远程解密 Arkose Token 求解接口列表
    pub proxy_url_list: Vec<String>,           // 与官方通信时使用的主请求代理地址列表
    pub sentinel_proxy_url_list: Vec<String>,  // Sentinel 握手请求时的特定隔离代理列表
    pub export_proxy_url: Option<String>,      // 多模态资源下载的出口防泄密代理
    pub file_host: Option<String>,             // 静态文件服务器 Host 覆盖
    pub voice_host: Option<String>,            // 语音服务 Host 覆盖
    pub impersonate_list: Vec<String>,         // 拟态浏览器套接字握手列表 (如 chrome100)
    pub user_agents_list: Vec<String>,         // 随机 User-Agent 请求头列表
    pub turnstile_solver_url: Option<String>,  // 远程解密 Cloudflare Turnstile 求解器地址
    pub history_disabled: bool,                // 会话是否不在官网存档 (默认为 true 不保存)
    pub pow_difficulty: String,                // 设定的 POW 最小解密前缀难度 (默认 "000032")
    pub retry_times: usize,                    // 会话出错时的轮询重试次数限制
    pub conversation_only: bool,               // 是否直接发送会话而跳过 Sentinel 检查
    pub enable_limit: bool,                    // 是否在本地启用频控限流拦截 (保护账号)
    pub upload_by_url: bool,                   // 问答正文中若含图片 URL 是否自动识别上传
    pub check_model: bool,                     // 是否校验客户端传入模型的存在性
    pub scheduled_refresh: bool,               // 是否启动后台定时自动刷新 Token 令牌的任务
    pub random_token: bool,                    // 账号池选用策略 (true 为随机抽取，false 为顺序轮询)
    pub oai_language: String,                  // 传给 OpenAI 后端的首选界面显示语言
    pub enable_gateway: bool,                  // 是否允许运行官网镜像网关界面 (tokens/login/web UI)
    pub auto_seed: bool,                       // 官网镜像下是否允许通过 seed 隔离并随机绑定账号
    pub force_no_history: bool,                // 是否强行不记录会话历史
    pub no_sentinel: bool,                     // 是否直接剔除 Sentinel 头参数
    pub cf_file_url: Option<String>,           // 使用 Cloudflare Workers 代理抓取下载多模态资源
    pub cf_clearance: Option<String>,          // Cloudflare clearance token (用于绕过 Grok Web 反检测)
    pub flaresolverr_url: Option<String>,      // FlareSolverr 接口网关地址 (用于自动挑战获取 cf_clearance)
    pub version: String,                       // 程序读取 version.txt 的当前版本号
}

/// 辅助解析字符串变量是否为 true-like 布尔类型
fn parse_bool(val: &str) -> bool {
    let lower = val.to_lowercase();
    lower == "true" || lower == "1" || lower == "t" || lower == "y" || lower == "yes"
}

/// 辅助根据英文逗号分隔环境变量字符串为 String 数组
fn split_comma(val: &str) -> Vec<String> {
    if val.is_empty() {
        Vec::new()
    } else {
        val.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    }
}

/// 将 JSON 数组格式的环境变量 (例如: ["chrome100","chrome120"]) 还原解析为 Vec<String>
fn parse_json_array(val: &str) -> Vec<String> {
    serde_json::from_str(val).unwrap_or_else(|_| Vec::new())
}

impl Config {
    /// 从 `.env` 配置文件或系统环境变量中加载所有的环境参数
    pub fn load() -> Self {
        // 尝试加载本地根目录的 .env 文件并注入为进程变量，忽略不存在的错误
        let _ = dotenvy::dotenv();

        // 获取版本号信息
        let version = fs::read_to_string("version.txt")
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "0.1.0".to_string());

        let api_prefix = env::var("API_PREFIX").ok().filter(|s| !s.trim().is_empty());
        
        let authorization = env::var("AUTHORIZATION").unwrap_or_default();
        let authorization_list = split_comma(&authorization);

        let chatgpt_base_url = env::var("CHATGPT_BASE_URL").unwrap_or_else(|_| "https://chatgpt.com".to_string());
        let chatgpt_base_url_list = split_comma(&chatgpt_base_url);

        let auth_key = env::var("AUTH_KEY").ok().filter(|s| !s.trim().is_empty());
        let x_sign = env::var("X_SIGN").ok().filter(|s| !s.trim().is_empty());

        let ark0se_token_url = env::var("ARK0SE_TOKEN_URL")
            .unwrap_or_else(|_| env::var("ARKOSE_TOKEN_URL").unwrap_or_default());
        let ark0se_token_url_list = split_comma(&ark0se_token_url);

        let proxy_url = env::var("PROXY_URL").unwrap_or_default();
        let proxy_url_list = split_comma(&proxy_url);

        let sentinel_proxy_url = env::var("SENTINEL_PROXY_URL").unwrap_or_default();
        let sentinel_proxy_url_list = split_comma(&sentinel_proxy_url);

        let export_proxy_url = env::var("EXPORT_PROXY_URL").ok().filter(|s| !s.trim().is_empty());
        let file_host = env::var("FILE_HOST").ok().filter(|s| !s.trim().is_empty());
        let voice_host = env::var("VOICE_HOST").ok().filter(|s| !s.trim().is_empty());

        let impersonate_list_str = env::var("IMPERSONATE").unwrap_or_else(|_| "[]".to_string());
        let impersonate_list = parse_json_array(&impersonate_list_str);

        let user_agents_list_str = env::var("USER_AGENTS").unwrap_or_else(|_| "[]".to_string());
        let user_agents_list = parse_json_array(&user_agents_list_str);

        let turnstile_solver_url = env::var("TURNSTILE_SOLVER_URL").ok().filter(|s| !s.trim().is_empty());
        let cf_file_url = env::var("CF_FILE_URL").ok().filter(|s| !s.trim().is_empty());
        let cf_clearance = env::var("CF_CLEARANCE").ok().filter(|s| !s.trim().is_empty());
        let flaresolverr_url = env::var("FLARESOLVERR_URL").ok().filter(|s| !s.trim().is_empty());

        let history_disabled = env::var("HISTORY_DISABLED")
            .map(|v| parse_bool(&v))
            .unwrap_or(true);

        let pow_difficulty = env::var("POW_DIFFICULTY").unwrap_or_else(|_| "000032".to_string());

        let retry_times = env::var("RETRY_TIMES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);

        let conversation_only = env::var("CONVERSATION_ONLY")
            .map(|v| parse_bool(&v))
            .unwrap_or(false);

        let enable_limit = env::var("ENABLE_LIMIT")
            .map(|v| parse_bool(&v))
            .unwrap_or(true);

        let upload_by_url = env::var("UPLOAD_BY_URL")
            .map(|v| parse_bool(&v))
            .unwrap_or(false);

        let check_model = env::var("CHECK_MODEL")
            .map(|v| parse_bool(&v))
            .unwrap_or(false);

        let scheduled_refresh = env::var("SCHEDULED_REFRESH")
            .map(|v| parse_bool(&v))
            .unwrap_or(false);

        let random_token = env::var("RANDOM_TOKEN")
            .map(|v| parse_bool(&v))
            .unwrap_or(true);

        let oai_language = env::var("OAI_LANGUAGE").unwrap_or_else(|_| "zh-CN".to_string());

        let enable_gateway = env::var("ENABLE_GATEWAY")
            .map(|v| parse_bool(&v))
            .unwrap_or(false);

        let auto_seed = env::var("AUTO_SEED")
            .map(|v| parse_bool(&v))
            .unwrap_or(true);

        let force_no_history = env::var("FORCE_NO_HISTORY")
            .map(|v| parse_bool(&v))
            .unwrap_or(false);

        let no_sentinel = env::var("NO_SENTINEL")
            .map(|v| parse_bool(&v))
            .unwrap_or(false);

        let config = Self {
            api_prefix,
            authorization_list,
            chatgpt_base_url_list,
            auth_key,
            x_sign,
            ark0se_token_url_list,
            proxy_url_list,
            sentinel_proxy_url_list,
            export_proxy_url,
            file_host,
            voice_host,
            impersonate_list,
            user_agents_list,
            turnstile_solver_url,
            history_disabled,
            pow_difficulty,
            retry_times,
            conversation_only,
            enable_limit,
            upload_by_url,
            check_model,
            scheduled_refresh,
            random_token,
            oai_language,
            enable_gateway,
            auto_seed,
            force_no_history,
            no_sentinel,
            cf_file_url,
            cf_clearance,
            flaresolverr_url,
            version,
        };

        // 打印系统装载配置成功日志
        config.print_log();

        config
    }

    /// 格式化控制台输出当前系统运行的环境变量明细日志
    fn print_log(&self) {
        info!("------------------------------------------------------------");
        info!("Chat2Api Rust {} | https://github.com/lanqian528/chat2api", self.version);
        info!("------------------------------------------------------------");
        info!("系统装载的环境变量明细如下:");
        info!("------------------------- 安全配置 -------------------------");
        info!("API_PREFIX:        {:?}", self.api_prefix);
        info!("AUTHORIZATION:     {:?}", self.authorization_list);
        info!("AUTH_KEY:          {:?}", self.auth_key);
        info!("------------------------- 网络请求 -------------------------");
        info!("CHATGPT_BASE_URL:  {:?}", self.chatgpt_base_url_list);
        info!("PROXY_URL:         {:?}", self.proxy_url_list);
        info!("EXPORT_PROXY_URL:  {:?}", self.export_proxy_url);
        info!("FILE_HOST:         {:?}", self.file_host);
        info!("VOICE_HOST:        {:?}", self.voice_host);
        info!("IMPERSONATE:       {:?}", self.impersonate_list);
        info!("USER_AGENTS:       {:?}", self.user_agents_list);
        info!("CF_FILE_URL:       {:?}", self.cf_file_url);
        info!("CF_CLEARANCE:      {}", self.cf_clearance.as_ref().map(|s| !s.is_empty()).unwrap_or(false));
        info!("FLARESOLVERR_URL:  {:?}", self.flaresolverr_url);
        info!("---------------------- 接口功能参数 -----------------------");
        info!("HISTORY_DISABLED:  {}", self.history_disabled);
        info!("POW_DIFFICULTY:    {}", self.pow_difficulty);
        info!("RETRY_TIMES:       {}", self.retry_times);
        info!("CONVERSATION_ONLY: {}", self.conversation_only);
        info!("ENABLE_LIMIT:      {}", self.enable_limit);
        info!("UPLOAD_BY_URL:     {}", self.upload_by_url);
        info!("CHECK_MODEL:       {}", self.check_model);
        info!("SCHEDULED_REFRESH: {}", self.scheduled_refresh);
        info!("RANDOM_TOKEN:      {}", self.random_token);
        info!("OAI_LANGUAGE:      {}", self.oai_language);
        info!("------------------------- 官网网关 -------------------------");
        info!("ENABLE_GATEWAY:    {}", self.enable_gateway);
        info!("AUTO_SEED:         {}", self.auto_seed);
        info!("FORCE_NO_HISTORY:  {}", self.force_no_history);
        info!("------------------------------------------------------------");
    }
}
