use std::env;
use std::fs;
use log::info;

#[derive(Debug, Clone)]
pub struct Config {
    pub api_prefix: Option<String>,
    pub authorization_list: Vec<String>,
    pub chatgpt_base_url_list: Vec<String>,
    pub auth_key: Option<String>,
    pub x_sign: Option<String>,
    pub ark0se_token_url_list: Vec<String>,
    pub proxy_url_list: Vec<String>,
    pub sentinel_proxy_url_list: Vec<String>,
    pub export_proxy_url: Option<String>,
    pub file_host: Option<String>,
    pub voice_host: Option<String>,
    pub impersonate_list: Vec<String>,
    pub user_agents_list: Vec<String>,
    pub turnstile_solver_url: Option<String>,
    pub history_disabled: bool,
    pub pow_difficulty: String,
    pub retry_times: usize,
    pub conversation_only: bool,
    pub enable_limit: bool,
    pub upload_by_url: bool,
    pub check_model: bool,
    pub scheduled_refresh: bool,
    pub random_token: bool,
    pub oai_language: String,
    pub enable_gateway: bool,
    pub auto_seed: bool,
    pub force_no_history: bool,
    pub no_sentinel: bool,
    pub version: String,
}

fn parse_bool(val: &str) -> bool {
    let lower = val.to_lowercase();
    lower == "true" || lower == "1" || lower == "t" || lower == "y" || lower == "yes"
}

fn split_comma(val: &str) -> Vec<String> {
    if val.is_empty() {
        Vec::new()
    } else {
        val.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    }
}

fn parse_json_array(val: &str) -> Vec<String> {
    serde_json::from_str(val).unwrap_or_else(|_| Vec::new())
}

impl Config {
    pub fn load() -> Self {
        // 尝试加载 .env 文件，忽略错误（如果不存在）
        let _ = dotenvy::dotenv();

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
            version,
        };

        config.print_log();

        config
    }

    fn print_log(&self) {
        info!("------------------------------------------------------------");
        info!("Chat2Api Rust {} | https://github.com/lanqian528/chat2api", self.version);
        info!("------------------------------------------------------------");
        info!("Environment variables:");
        info!("------------------------- Security -------------------------");
        info!("API_PREFIX:        {:?}", self.api_prefix);
        info!("AUTHORIZATION:     {:?}", self.authorization_list);
        info!("AUTH_KEY:          {:?}", self.auth_key);
        info!("------------------------- Request --------------------------");
        info!("CHATGPT_BASE_URL:  {:?}", self.chatgpt_base_url_list);
        info!("PROXY_URL:         {:?}", self.proxy_url_list);
        info!("EXPORT_PROXY_URL:  {:?}", self.export_proxy_url);
        info!("FILE_HOST:         {:?}", self.file_host);
        info!("VOICE_HOST:        {:?}", self.voice_host);
        info!("IMPERSONATE:       {:?}", self.impersonate_list);
        info!("USER_AGENTS:       {:?}", self.user_agents_list);
        info!("---------------------- Functionality -----------------------");
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
        info!("------------------------- Gateway --------------------------");
        info!("ENABLE_GATEWAY:    {}", self.enable_gateway);
        info!("AUTO_SEED:         {}", self.auto_seed);
        info!("FORCE_NO_HISTORY:  {}", self.force_no_history);
        info!("------------------------------------------------------------");
    }
}
