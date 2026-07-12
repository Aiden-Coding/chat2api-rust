use rquest::{Client as ReqwestClient, Proxy, tls::Impersonate};
use std::time::Duration;

pub fn create_client(proxy_url: Option<&str>, impersonate_name: &str) -> Result<ReqwestClient, rquest::Error> {
    let impersonate = match impersonate_name.to_lowercase().as_str() {
        "chrome100" | "chrome99" | "chrome101" | "chrome110" => Impersonate::Chrome100,
        "chrome104" => Impersonate::Chrome104,
        "chrome107" => Impersonate::Chrome107,
        "chrome116" => Impersonate::Chrome116,
        "chrome119" => Impersonate::Chrome119,
        "chrome120" | "chrome123" => Impersonate::Chrome120,
        "safari15_3" => Impersonate::Safari15_3,
        "edge99" | "edge101" => Impersonate::Edge101,
        _ => Impersonate::Safari15_3,
    };

    let mut builder = ReqwestClient::builder()
        .impersonate(impersonate)
        .danger_accept_invalid_certs(true);

    if let Some(proxy_str) = proxy_url {
        if !proxy_str.is_empty() {
            if let Ok(proxy) = Proxy::all(proxy_str) {
                builder = builder.proxy(proxy);
            }
        }
    }

    builder.build()
}
