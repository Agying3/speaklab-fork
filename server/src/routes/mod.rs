//! 路由表。
//!
//! 所有接口挂在 `/api/v1` 下。版本号写进路径是为了以后改结构时
//! 老版本还能继续服务，不用逼客户端一起升级。

pub mod health;
pub mod practices;

use axum::routing::{get, post};
use axum::Router;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/health", get(health::health))
        .route("/api/v1/meta", get(health::meta))
        .route(
            "/api/v1/practices",
            post(practices::create).get(practices::list),
        )
        .route("/api/v1/practices/stats", get(practices::stats))
        .route("/api/v1/asr", post(practices::transcribe))
}

/// 从 `Authorization: Bearer xxx` 或 `X-SpeakLab-Token` 里取出令牌。
///
/// 两种都支持是因为前端要同时服务浏览器和命令行调用，
/// 后者用自定义头更方便。
pub fn bearer_token(headers: &axum::http::HeaderMap) -> Option<String> {
    if let Some(v) = headers.get(axum::http::header::AUTHORIZATION) {
        if let Ok(s) = v.to_str() {
            if let Some(rest) = s.strip_prefix("Bearer ") {
                return Some(rest.trim().to_owned());
            }
        }
    }
    headers
        .get("x-speaklab-token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn reads_bearer_header() {
        let mut h = HeaderMap::new();
        h.insert("authorization", HeaderValue::from_static("Bearer abc123"));
        assert_eq!(bearer_token(&h).as_deref(), Some("abc123"));
    }

    #[test]
    fn reads_custom_header() {
        let mut h = HeaderMap::new();
        h.insert("x-speaklab-token", HeaderValue::from_static("xyz"));
        assert_eq!(bearer_token(&h).as_deref(), Some("xyz"));
    }

    #[test]
    fn missing_token_is_none() {
        assert!(bearer_token(&HeaderMap::new()).is_none());
    }

    #[test]
    fn other_auth_schemes_are_ignored() {
        let mut h = HeaderMap::new();
        h.insert("authorization", HeaderValue::from_static("Basic abc"));
        assert!(bearer_token(&h).is_none());
    }
}
