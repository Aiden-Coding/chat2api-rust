use wreq::{Client, Proxy};
use wreq_util::Emulation;

/// 根据给定的代理 URL 与浏览器 Emulation 类型构建 wreq (BoringSSL Chrome) 的 HTTP 客户端
/// wreq 使用 BoringSSL（与 Chrome 相同的 TLS 库），能够产生与真实浏览器完全一致的 JA3/JA4 指纹
pub fn create_client(proxy_url: Option<&str>, impersonate_name: &str) -> Result<Client, wreq::Error> {
    let emulation = name_to_emulation(impersonate_name);

    let mut builder = Client::builder()
        .emulation(emulation);

    if let Some(proxy_str) = proxy_url {
        if !proxy_str.is_empty() {
            if let Ok(proxy) = Proxy::all(proxy_str) {
                builder = builder.proxy(proxy);
            }
        }
    }

    builder.build()
}

/// 为 Grok Web 模式 (grok.com) 创建专用客户端
/// 使用 Chrome136 Emulation，不跳过 TLS 证书校验
/// 以保持完整的 BoringSSL Chrome JA3/JA4 指纹，通过 Cloudflare 反机器人检测
pub fn create_grok_web_client(proxy_url: Option<&str>) -> Result<Client, wreq::Error> {
    // Chrome136 是目前最接近当前真实浏览器的版本，且 Cloudflare 对其有良好支持
    let order = vec![
        wreq::header::HeaderName::from_static("host"),
        wreq::header::HeaderName::from_static("sec-ch-ua"),
        wreq::header::HeaderName::from_static("sec-ch-ua-mobile"),
        wreq::header::HeaderName::from_static("sec-ch-ua-platform"),
        wreq::header::HeaderName::from_static("upgrade-insecure-requests"),
        wreq::header::HeaderName::from_static("user-agent"),
        wreq::header::HeaderName::from_static("accept"),
        wreq::header::HeaderName::from_static("sec-fetch-site"),
        wreq::header::HeaderName::from_static("sec-fetch-mode"),
        wreq::header::HeaderName::from_static("sec-fetch-user"),
        wreq::header::HeaderName::from_static("sec-fetch-dest"),
        wreq::header::HeaderName::from_static("accept-encoding"),
        wreq::header::HeaderName::from_static("accept-language"),
        wreq::header::HeaderName::from_static("priority"),
        wreq::header::HeaderName::from_static("baggage"),
        wreq::header::HeaderName::from_static("content-type"),
        wreq::header::HeaderName::from_static("origin"),
        wreq::header::HeaderName::from_static("referer"),
        wreq::header::HeaderName::from_static("x-statsig-id"),
        wreq::header::HeaderName::from_static("x-xai-request-id"),
        wreq::header::HeaderName::from_static("sec-ch-ua-model"),
        wreq::header::HeaderName::from_static("sec-ch-ua-arch"),
        wreq::header::HeaderName::from_static("sec-ch-ua-bitness"),
        wreq::header::HeaderName::from_static("cookie"),
    ];

    let mut builder = Client::builder()
        .emulation(Emulation::Chrome136)
        .headers_order(order);

    if let Some(proxy_str) = proxy_url {
        if !proxy_str.is_empty() {
            if let Ok(proxy) = Proxy::all(proxy_str) {
                builder = builder.proxy(proxy);
            }
        }
    }

    builder.build()
}

/// 将字符串名称映射到 wreq-util Emulation 枚举值
pub fn name_to_emulation(name: &str) -> Emulation {
    match name.to_lowercase().as_str() {
        "chrome100" | "chrome99" | "chrome101" => Emulation::Chrome100,
        "chrome104" => Emulation::Chrome104,
        "chrome107" => Emulation::Chrome107,
        "chrome110" => Emulation::Chrome110,
        "chrome116" => Emulation::Chrome116,
        "chrome119" => Emulation::Chrome119,
        "chrome120" => Emulation::Chrome120,
        "chrome123" => Emulation::Chrome123,
        "chrome124" => Emulation::Chrome124,
        "chrome126" => Emulation::Chrome126,
        "chrome127" => Emulation::Chrome127,
        "chrome128" => Emulation::Chrome128,
        "chrome129" => Emulation::Chrome129,
        "chrome130" => Emulation::Chrome130,
        "chrome131" => Emulation::Chrome131,
        "chrome132" => Emulation::Chrome132,
        "chrome133" => Emulation::Chrome133,
        "chrome134" => Emulation::Chrome134,
        "chrome135" => Emulation::Chrome135,
        "chrome136" => Emulation::Chrome136,
        "chrome137" => Emulation::Chrome137,
        "safari15_3" | "safari15" => Emulation::Safari15_3,
        "safari17" | "safari17_0" => Emulation::Safari17_0,
        "safari18" => Emulation::Safari18,
        "edge101" | "edge" => Emulation::Edge101,
        "edge122" => Emulation::Edge122,
        "edge127" => Emulation::Edge127,
        "edge131" => Emulation::Edge131,
        "edge134" => Emulation::Edge134,
        _ => Emulation::Chrome136, // 默认使用 Chrome136 (最新且稳定)
    }
}
