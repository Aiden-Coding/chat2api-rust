use actix_web::{middleware, web, App, HttpServer};
use tera::Tera;
use log::info;

use chat2api::config::Config;
use chat2api::globals::AppState;
use chat2api::chatgpt::auth::refresh_all_tokens;
use chat2api::api::routes::{send_conversation, upload_html, upload_post, clear_tokens, error_tokens, add_token, clear_seed_tokens};

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // 初始化日志
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // 加载配置
    let config = Config::load();
    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(5005);

    // 初始化全局共享状态
    let state = AppState::new(&config);

    // 后台定时刷新 RefreshToken
    if config.scheduled_refresh {
        let state_clone = state.clone();
        let config_clone = config.clone();
        tokio::spawn(async move {
            info!("Scheduled Token Refresh Task started.");
            
            // 每次启动先进行一次非强制刷新
            refresh_all_tokens(&state_clone, &config_clone, false).await;
            
            loop {
                // 每隔 4 天强制刷新一次
                // 用简单的 Sleep 4 天模拟 (4 * 24 * 3600 秒)
                tokio::time::sleep(std::time::Duration::from_secs(4 * 24 * 3600)).await;
                info!("Scheduled forced token refresh triggered.");
                refresh_all_tokens(&state_clone, &config_clone, true).await;
            }
        });
    }

    // 初始化 Tera 模板引擎
    let tera = match Tera::new("templates/**/*") {
        Ok(t) => t,
        Err(e) => {
            log::error!("Failed to parse templates: {:?}", e);
            // 如果不存在，使用默认空 Tera 实例防崩溃
            Tera::default()
        }
    };

    let state_data = web::Data::new(state);
    let config_data = web::Data::new(config.clone());
    let tera_data = web::Data::new(tera);

    let prefix = config.api_prefix.clone().unwrap_or_default();
    let display_prefix = if prefix.is_empty() { "none".to_string() } else { format!("/{}", prefix) };

    info!("Starting Actix-web server on http://0.0.0.0:{}", port);
    info!("Api Prefix route configured to: {}", display_prefix);

    HttpServer::new(move || {
        let prefix_path = if prefix.is_empty() {
            "".to_string()
        } else {
            format!("/{}", prefix)
        };

        App::new()
            .wrap(middleware::Logger::default())
            .wrap(
                actix_cors::Cors::default()
                    .allow_any_origin()
                    .allow_any_method()
                    .allow_any_header()
                    .max_age(3600)
            )
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
            .app_data(state_data.clone())
            .app_data(config_data.clone())
            .app_data(tera_data.clone())
            .service(
                web::scope(&prefix_path)
                    .service(send_conversation)
                    .service(upload_html)
                    .service(upload_post)
                    .service(clear_tokens)
                    .service(error_tokens)
                    .service(add_token)
                    .service(clear_seed_tokens)
            )
    })
    .bind(("0.0.0.0", port))?
    .run()
    .await
}
