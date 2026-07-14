use serde::{Deserialize, Serialize};
use log::{info, warn, error};

#[derive(Serialize)]
struct FlareSolverrRequest {
    cmd: &'static str,
    url: &'static str,
    #[serde(rename = "maxTimeout")]
    max_timeout: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    proxy: Option<FlareSolverrProxy>,
}

#[derive(Serialize)]
struct FlareSolverrProxy {
    url: String,
}

#[derive(Deserialize, Debug)]
struct FlareSolverrCookie {
    name: String,
    value: String,
    #[serde(rename = "domain")]
    _domain: Option<String>,
}

#[derive(Deserialize, Debug)]
struct FlareSolverrSolution {
    #[serde(rename = "url")]
    _url: String,
    #[serde(rename = "status")]
    _status: u16,
    cookies: Vec<FlareSolverrCookie>,
    #[serde(rename = "userAgent")]
    user_agent: String,
}

#[derive(Deserialize, Debug)]
struct FlareSolverrResponse {
    status: String,
    message: String,
    solution: Option<FlareSolverrSolution>,
}

/// Request cf_clearance and user_agent from FlareSolverr
pub async fn solve_cf_clearance(
    flaresolverr_url: &str,
    proxy_url: Option<&str>,
) -> Option<(String, String)> {
    let client = wreq::Client::new();
    let fs_endpoint = format!("{}/v1", flaresolverr_url.trim_end_matches('/'));

    let proxy = proxy_url.map(|p| FlareSolverrProxy { url: p.to_string() });
    let req_payload = FlareSolverrRequest {
        cmd: "request.get",
        url: "https://grok.com",
        max_timeout: 60000,
        proxy,
    };

    info!("Calling FlareSolverr to solve Grok Cloudflare challenge at: {}", fs_endpoint);

    let resp_res = client.post(&fs_endpoint)
        .header("content-type", "application/json")
        .json(&req_payload)
        .send()
        .await;

    match resp_res {
        Ok(resp) => {
            if !resp.status().is_success() {
                warn!("FlareSolverr returned unsuccessful HTTP status: {}", resp.status());
                return None;
            }
            match resp.json::<FlareSolverrResponse>().await {
                Ok(fs_resp) => {
                    if fs_resp.status != "ok" {
                        warn!("FlareSolverr returned status: {} with message: {}", fs_resp.status, fs_resp.message);
                        return None;
                    }
                    if let Some(solution) = fs_resp.solution {
                        let cf_clearance = solution.cookies.iter()
                            .find(|c| c.name == "cf_clearance")
                            .map(|c| c.value.clone());
                        
                        if let Some(cf) = cf_clearance {
                            info!("FlareSolverr solved successfully! User-Agent: {}", solution.user_agent);
                            return Some((cf, solution.user_agent));
                        } else {
                            warn!("FlareSolverr succeeded but no cf_clearance cookie was found in response.");
                        }
                    } else {
                        warn!("FlareSolverr response did not contain solution details.");
                    }
                }
                Err(e) => {
                    error!("Failed to parse FlareSolverr JSON response: {:?}", e);
                }
            }
        }
        Err(e) => {
            error!("FlareSolverr request failed: {:?}", e);
        }
    }

    None
}
