use actix_web::{middleware, web, App, HttpServer};
use tera::Tera;
use log::info;

use chat2api::config::Config;
use chat2api::globals::AppState;
use chat2api::chatgpt::auth::refresh_all_tokens;
use chat2api::api::routes::{send_conversation, upload_html, upload_post, clear_tokens, error_tokens, get_token_list, delete_tokens, add_token, clear_seed_tokens};

/// Actix-web 服务运行主入口函数
#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // 1. 初始化 env_logger 日志组件，默认日志过滤级别为 info
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // 2. 加载系统的全局变量配置项 (从系统变量或本地 .env)
    let config = Config::load();
    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(5005); // 默认监听 5005 端口

    // 3. 初始化全局共享的读写锁 App 状态容器 (AppState)
    let state = AppState::new(&config);

    // 4. 判断是否开启后台定时自动刷新 RefreshToken 的异步任务
    if config.scheduled_refresh {
        let state_clone = state.clone();
        let config_clone = config.clone();
        tokio::spawn(async move {
            info!("异步定时 RefreshToken 自动轮询刷新任务已经成功启动。");
            
            // 每次启动程序，非强制性刷新一次 (检测并预热缓存)
            refresh_all_tokens(&state_clone, &config_clone, false).await;
            
            loop {
                // 每隔 4 天时间在后台睡眠并触发一次强制刷新
                tokio::time::sleep(std::time::Duration::from_secs(4 * 24 * 3600)).await;
                info!("定时强制刷新任务触发：正在强制更新所有已注册 RefreshToken 对应的 AccessToken 缓存。");
                refresh_all_tokens(&state_clone, &config_clone, true).await;
            }
        });
    }

    let tera = match Tera::new("src/templates/**/*") {
        Ok(t) => t,
        Err(e) => {
            log::error!("解析模板 src/templates 文件时遇到致命错误: {:?}", e);
            Tera::default() // 模板加载失败则使用空 Tera 以防止项目启动崩溃
        }
    };

    // 6. 将各全局共享配置包装为 actix_web 的 web::Data 数据项以供路由取用
    let state_data = web::Data::new(state);
    let config_data = web::Data::new(config.clone());
    let tera_data = web::Data::new(tera);

    let prefix = config.api_prefix.clone().unwrap_or_default();
    let display_prefix = if prefix.is_empty() { "无限制".to_string() } else { format!("/{}", prefix) };

    info!("正在启动高性能 Actix-web 服务在：http://0.0.0.0:{}", port);
    info!("路由前缀密码防护挂载为: {}", display_prefix);

    // 7. 绑定端口并构建 HTTP 多线程服务器实例
    HttpServer::new(move || {
        let prefix_path = if prefix.is_empty() {
            "".to_string()
        } else {
            format!("/{}", prefix)
        };

        App::new()
            // 挂载默认日志输出拦截器
            .wrap(middleware::Logger::default())
            // 挂载跨域 CORS 中间件，允许全源、全请求方法和全请求头访问
            .wrap(
                actix_cors::Cors::default()
                    .allow_any_origin()
                    .allow_any_method()
                    .allow_any_header()
                    .max_age(3600)
            )
            // 自定义全局的 JSON 反序列化格式错误捕获与转换
            .app_data(
                web::JsonConfig::default()
                    .error_handler(|err, _req| {
                        let msg = format!("{}", err);
                        actix_web::error::InternalError::from_response(
                            err,
                            actix_web::HttpResponse::BadRequest().json(serde_json::json!({
                                "error": { "message": msg, "type": "invalid_request_error" }
                            })),
                        )
                        .into()
                    })
            )
            // 挂载各全局共享状态
            .app_data(state_data.clone())
            .app_data(config_data.clone())
            .app_data(tera_data.clone())
            // 挂载主路由作用域
            .service(
                web::scope(&prefix_path)
                    .service(send_conversation)
                    .service(upload_html)
                    .service(upload_post)
                    .service(clear_tokens)
                    .service(error_tokens)
                    .service(get_token_list)
                    .service(delete_tokens)
                    .service(add_token)
                    .service(clear_seed_tokens)
            )
    })
    .bind(("0.0.0.0", port))?
    .run()
    .await
}
