// ChatGPT 官网接口逆向解算及安全加密模块入口
pub mod pow;       // 工作量证明 (Proof of Work) 算法求解
pub mod turnstile; // Cloudflare Turnstile 本地解算器
pub mod auth;      // AccessToken 验证与 RefreshToken 定时刷新
pub mod client;    // Impersonate (拟态套接字) 客户端创建器
pub mod service;   // Sentinel 握手与会话构建业务服务核心
pub mod format;    // SSE 响应流解析与 OpenAI 协议流转换格式化
