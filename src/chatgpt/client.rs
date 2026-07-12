use rquest::{Client as ReqwestClient, Proxy, tls::Impersonate};
use std::time::Duration;

pub fn create_client(proxy_url: Option<&str>) -> Result<ReqwestClient, rquest::Error> {
    let mut builder = ReqwestClient::builder()
        .impersonate(Impersonate::Chrome120)
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
