//! Built-in local authentication
//!
//! Handles:
//! - Built-in local authentication
//! - Session management
//! - Authentication middleware

mod local;
mod middleware;
pub mod session;

use axum::http::HeaderMap;
use std::net::IpAddr;
use std::net::SocketAddr;

pub use local::{LocalAuthService, auth_router, ensure_local_auth_config};
pub use middleware::{
    CurrentOAuthAccess, CurrentUser, OAuthAccess, ScopePolicy, require_app_auth, require_auth,
    require_auth_scopes, require_auth_scopes_with_policy, require_metrics_auth,
    require_session_auth,
};
pub use session::{Session, create_session_token, verify_session_token};

fn forwarded_client_ip(headers: &HeaderMap, trusted_proxy_ips: &[IpAddr]) -> Option<IpAddr> {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        // Walk from right to left and skip trusted proxy hops; the first non-trusted
        // address is the closest known end-user hop in layered proxy deployments.
        .and_then(|value| {
            value
                .split(',')
                .rev()
                .map(str::trim)
                .filter(|candidate| !candidate.is_empty())
                .filter_map(|candidate| candidate.parse::<IpAddr>().ok())
                .find(|candidate| !trusted_proxy_ips.contains(candidate))
        })
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|value| value.to_str().ok())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .and_then(|value| value.parse::<IpAddr>().ok())
                .filter(|candidate| !trusted_proxy_ips.contains(candidate))
        })
}

fn request_client_identifier(
    peer_addr: SocketAddr,
    headers: &HeaderMap,
    trusted_proxy_ips: &[IpAddr],
) -> String {
    let peer_ip = peer_addr.ip();
    if trusted_proxy_ips.contains(&peer_ip) {
        if let Some(client_ip) = forwarded_client_ip(headers, trusted_proxy_ips) {
            return client_ip.to_string().chars().take(128).collect::<String>();
        }
    }
    peer_ip.to_string().chars().take(128).collect::<String>()
}

pub(crate) async fn check_auth_rate_limit(
    limiter: &crate::federation::RateLimiter,
    peer_addr: Option<SocketAddr>,
    headers: &HeaderMap,
    trusted_proxy_ips: &[IpAddr],
    endpoint: &str,
) -> Result<(), crate::error::AppError> {
    let peer_addr = peer_addr.ok_or_else(|| {
        crate::error::AppError::internal(
            "missing connection peer address for auth rate limiting; configure connect info",
        )
    })?;
    let mut key = format!(
        "{endpoint}:{}",
        request_client_identifier(peer_addr, headers, trusted_proxy_ips)
    );
    key.truncate(256);
    limiter.check_and_increment(&key).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use axum::http::header::HeaderName;

    #[test]
    fn request_client_identifier_uses_peer_address() {
        let peer = SocketAddr::from(([198, 51, 100, 7], 4242));
        let headers = HeaderMap::new();
        assert_eq!(
            request_client_identifier(peer, &headers, &[]),
            "198.51.100.7"
        );
    }

    #[test]
    fn request_client_identifier_uses_forwarded_ip_for_trusted_proxy() {
        let peer = SocketAddr::from(([127, 0, 0, 1], 8080));
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-forwarded-for"),
            HeaderValue::from_static("198.51.100.7, 203.0.113.10"),
        );
        let trusted_proxy_ips = vec![IpAddr::from([127, 0, 0, 1])];

        assert_eq!(
            request_client_identifier(peer, &headers, &trusted_proxy_ips),
            "203.0.113.10"
        );
    }

    #[test]
    fn request_client_identifier_skips_trusted_proxy_hops_in_forwarded_for() {
        let peer = SocketAddr::from(([10, 0, 0, 9], 8080));
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-forwarded-for"),
            HeaderValue::from_static("198.51.100.7, 10.0.0.7, 10.0.0.8"),
        );
        let trusted_proxy_ips = vec![
            IpAddr::from([10, 0, 0, 9]),
            IpAddr::from([10, 0, 0, 8]),
            IpAddr::from([10, 0, 0, 7]),
        ];

        assert_eq!(
            request_client_identifier(peer, &headers, &trusted_proxy_ips),
            "198.51.100.7"
        );
    }

    #[test]
    fn request_client_identifier_ignores_forwarded_ip_for_untrusted_proxy() {
        let peer = SocketAddr::from(([203, 0, 113, 5], 8080));
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-forwarded-for"),
            HeaderValue::from_static("198.51.100.7"),
        );
        let trusted_proxy_ips = vec![IpAddr::from([127, 0, 0, 1])];

        assert_eq!(
            request_client_identifier(peer, &headers, &trusted_proxy_ips),
            "203.0.113.5"
        );
    }

    #[test]
    fn request_client_identifier_ignores_leftmost_spoofed_forwarded_for_entry() {
        let peer = SocketAddr::from(([127, 0, 0, 1], 8080));
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-forwarded-for"),
            HeaderValue::from_static("1.2.3.4, 198.51.100.42"),
        );
        let trusted_proxy_ips = vec![IpAddr::from([127, 0, 0, 1])];

        assert_eq!(
            request_client_identifier(peer, &headers, &trusted_proxy_ips),
            "198.51.100.42"
        );
    }

    #[tokio::test]
    async fn check_auth_rate_limit_rejects_missing_peer_address() {
        let limiter =
            crate::federation::RateLimiter::new(Some(1), Some(std::time::Duration::from_secs(60)));
        let headers = HeaderMap::new();

        let error = check_auth_rate_limit(&limiter, None, &headers, &[], "auth")
            .await
            .expect_err("missing peer address must fail closed");
        assert!(matches!(error, crate::error::AppError::Internal(_)));
    }

    #[tokio::test]
    async fn check_auth_rate_limit_rejects_excessive_requests() {
        let limiter =
            crate::federation::RateLimiter::new(Some(1), Some(std::time::Duration::from_secs(60)));
        let headers = HeaderMap::new();

        let peer = SocketAddr::from(([198, 51, 100, 9], 4567));
        check_auth_rate_limit(&limiter, Some(peer), &headers, &[], "auth")
            .await
            .expect("first request should pass");
        let blocked = check_auth_rate_limit(&limiter, Some(peer), &headers, &[], "auth").await;
        assert!(matches!(blocked, Err(crate::error::AppError::RateLimited)));
    }
}
