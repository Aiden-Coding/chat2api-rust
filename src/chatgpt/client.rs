use rquest::{Client as ReqwestClient, Proxy, tls::Impersonate};

/// 根据给定的代理 URL 与浏览器 Impersonate 指纹类型构建 rquest (仿 Chrome/Safari) 的 HTTP 客户端
/// proxy_url: 代理地址 (例如 http://127.0.0.1:7890)
/// impersonate_name: 混淆拟态浏览器的指纹名称
pub fn create_client(proxy_url: Option<&str>, impersonate_name: &str) -> Result<ReqwestClient, rquest::Error> {
    // 匹配并确定底层的 Impersonate 拟态指纹
    let impersonate = match impersonate_name.to_lowercase().as_str() {
        "chrome100" | "chrome99" | "chrome101" | "chrome110" => Impersonate::Chrome100,
        "chrome104" => Impersonate::Chrome104,
        "chrome107" => Impersonate::Chrome107,
        "chrome116" => Impersonate::Chrome116,
        "chrome119" => Impersonate::Chrome119,
        "chrome120" | "chrome123" | "chrome124" | "chrome125" | "chrome126" => Impersonate::Chrome120,
        "safari15_3" | "safari15" => Impersonate::Safari15_3,
        "safari17" | "safari17_0" => Impersonate::Safari17_0,
        "edge99" | "edge101" | "edge" => Impersonate::Edge101,
        _ => Impersonate::Chrome120, // 默认采用 Chrome 120 指纹（更现代）
    };

    let mut builder = ReqwestClient::builder()
        .impersonate(impersonate) // 设置拟态浏览器 SSL/JA3 握手参数
        .danger_accept_invalid_certs(true); // 允许跳过非法的 SSL/TLS 证书校验 (防止中间人代理捕获报错)

    // 如果指定了代理，则将其注入到 client 客户端构建器中
    if let Some(proxy_str) = proxy_url {
        if !proxy_str.is_empty() {
            if let Ok(proxy) = Proxy::all(proxy_str) {
                builder = builder.proxy(proxy);
            }
        }
    }

    builder.build()
}
