use axum::{extract::State, http::HeaderMap, Json};
use std::net::IpAddr;
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tracing::{error, warn};
use url::Url;

use crate::browser::BrowserManager;
use crate::error::AppError;
use crate::llm::GeminiClient;
use crate::models::{ScrapeData, ScrapeRequest, ScrapeResponse};

const API_KEY_HEADER: &str = "x-api-key";

pub struct AppState {
    pub browser: BrowserManager,
    pub llm_client: GeminiClient,
    pub api_key: String,
}

/// Constant-time comparison to prevent timing attacks
fn secure_compare(a: &str, b: &str) -> bool {
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

/// SSRF protection: blocks private/internal IPs
fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local()
                || v4.is_broadcast() || v4.is_unspecified()
        }
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified(),
    }
}

/// Validates URL and blocks SSRF attempts
fn validate_url(url_str: &str) -> Result<Url, AppError> {
    let url = Url::parse(url_str)
        .map_err(|e| AppError::InvalidRequest(format!("Invalid URL: {}", e)))?;

    match url.scheme() {
        "http" | "https" => {}
        s => return Err(AppError::InvalidRequest(format!("Invalid scheme: {}", s))),
    }

    let host = url
        .host_str()
        .ok_or_else(|| AppError::InvalidRequest("Missing host".to_string()))?;

    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_private_ip(&ip) {
            return Err(AppError::InvalidRequest("Private IP not allowed".to_string()));
        }
    }

    let host_lower = host.to_lowercase();
    if host_lower == "localhost" || host_lower.ends_with(".localhost") {
        return Err(AppError::InvalidRequest("Localhost not allowed".to_string()));
    }

    Ok(url)
}

pub async fn scrape_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<ScrapeRequest>,
) -> Result<Json<ScrapeResponse>, AppError> {
    let provided_key = headers
        .get(API_KEY_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !secure_compare(provided_key, &state.api_key) {
        warn!(url = %request.url, "Unauthorized");
        return Err(AppError::Unauthorized);
    }

    let validated_url = validate_url(&request.url)?;

    let (metadata, content) = state
        .browser
        .scrape_page(validated_url.as_str(), request.output_format)
        .await?;

    let (analysis_result, analysis_error) =
        if let Some(req) = request.analysis_request.as_ref() {
            match state.llm_client.analyze(&content, req).await {
                Ok(result) => (Some(result), None),
                Err(e) => {
                    error!(error = %e, "LLM analysis failed");
                    (None, Some(e.to_string()))
                }
            }
        } else {
            (None, None)
        };

    Ok(Json(ScrapeResponse::success(ScrapeData {
        metadata,
        content,
        analysis_result,
        analysis_error,
    })))
}

pub async fn health_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let stats = state.browser.stats().await;

    Json(serde_json::json!({
        "status": "healthy",
        "browser": {
            "max_concurrent": stats.max_concurrent,
            "available_slots": stats.available_slots,
            "idle_tabs": stats.idle_tabs,
            "active_tabs": stats.active_tabs
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== secure_compare ====================

    #[test]
    fn secure_compare_equal() {
        assert!(secure_compare("secret", "secret"));
    }

    #[test]
    fn secure_compare_different() {
        assert!(!secure_compare("secret", "wrong"));
    }

    #[test]
    fn secure_compare_empty() {
        assert!(secure_compare("", ""));
        assert!(!secure_compare("", "x"));
    }

    #[test]
    fn secure_compare_different_length() {
        assert!(!secure_compare("short", "longer_string"));
    }

    // ==================== is_private_ip ====================

    #[test]
    fn private_ip_loopback_v4() {
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        assert!(is_private_ip(&ip));
    }

    #[test]
    fn private_ip_loopback_v6() {
        let ip: IpAddr = "::1".parse().unwrap();
        assert!(is_private_ip(&ip));
    }

    #[test]
    fn private_ip_class_a() {
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        assert!(is_private_ip(&ip));
    }

    #[test]
    fn private_ip_class_b() {
        let ip: IpAddr = "172.16.0.1".parse().unwrap();
        assert!(is_private_ip(&ip));
    }

    #[test]
    fn private_ip_class_c() {
        let ip: IpAddr = "192.168.1.1".parse().unwrap();
        assert!(is_private_ip(&ip));
    }

    #[test]
    fn private_ip_link_local() {
        let ip: IpAddr = "169.254.1.1".parse().unwrap();
        assert!(is_private_ip(&ip));
    }

    #[test]
    fn public_ip_allowed() {
        let ip: IpAddr = "8.8.8.8".parse().unwrap();
        assert!(!is_private_ip(&ip));
    }

    // ==================== validate_url ====================

    #[test]
    fn validate_url_https() {
        assert!(validate_url("https://example.com").is_ok());
    }

    #[test]
    fn validate_url_http() {
        assert!(validate_url("http://example.com").is_ok());
    }

    #[test]
    fn validate_url_with_path() {
        assert!(validate_url("https://example.com/path/to/page").is_ok());
    }

    #[test]
    fn validate_url_with_query() {
        assert!(validate_url("https://example.com?q=test").is_ok());
    }

    #[test]
    fn validate_url_invalid_scheme_ftp() {
        assert!(validate_url("ftp://example.com").is_err());
    }

    #[test]
    fn validate_url_invalid_scheme_file() {
        assert!(validate_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn validate_url_invalid_format() {
        assert!(validate_url("not-a-url").is_err());
    }

    #[test]
    fn validate_url_localhost_blocked() {
        assert!(validate_url("http://localhost").is_err());
        assert!(validate_url("http://localhost:8080").is_err());
    }

    #[test]
    fn validate_url_localhost_subdomain_blocked() {
        assert!(validate_url("http://api.localhost").is_err());
    }

    #[test]
    fn validate_url_private_ip_blocked() {
        assert!(validate_url("http://127.0.0.1").is_err());
        assert!(validate_url("http://10.0.0.1").is_err());
        assert!(validate_url("http://192.168.1.1").is_err());
        assert!(validate_url("http://172.16.0.1").is_err());
    }

    #[test]
    fn validate_url_public_ip_allowed() {
        assert!(validate_url("http://8.8.8.8").is_ok());
    }
}
