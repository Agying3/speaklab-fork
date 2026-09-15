//! 统一错误类型。
//!
//! 所有 handler 返回 `Result<T, AppError>`，由这里的 `IntoResponse`
//! 决定 HTTP 状态码和响应体形状。响应体统一是
//! `{"error": {"code": "...", "message": "..."}}`，
//! 前端只要判断 `error` 字段在不在即可。

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("请求参数有误：{0}")]
    BadRequest(String),

    #[error("未授权")]
    Unauthorized,

    #[error("找不到 {0}")]
    NotFound(String),

    #[error("请求体过大，上限 {limit} 字节")]
    PayloadTooLarge { limit: usize },

    #[error("{0} 尚未配置")]
    NotImplemented(String),

    #[error("上游服务不可用：{0}")]
    Upstream(String),

    #[error("服务内部错误")]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    /// 稳定的机器可读错误码，前端用它做分支，不要匹配 message。
    pub fn code(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "bad_request",
            Self::Unauthorized => "unauthorized",
            Self::NotFound(_) => "not_found",
            Self::PayloadTooLarge { .. } => "payload_too_large",
            Self::NotImplemented(_) => "not_implemented",
            Self::Upstream(_) => "upstream_error",
            Self::Internal(_) => "internal",
        }
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::PayloadTooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
            Self::NotImplemented(_) => StatusCode::NOT_IMPLEMENTED,
            Self::Upstream(_) => StatusCode::BAD_GATEWAY,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();

        // 5xx 要留下完整原因，4xx 是调用方的问题，记简洁一点就够。
        if status.is_server_error() {
            tracing::error!(error = ?self, "请求处理失败");
        } else {
            tracing::debug!(error = %self, "请求被拒绝");
        }

        // 内部错误不把 anyhow 的链条暴露给调用方，避免泄漏路径等信息。
        let message = match &self {
            Self::Internal(_) => "服务内部错误，请查看服务端日志".to_owned(),
            other => other.to_string(),
        };

        let body = ErrorBody {
            error: ErrorDetail {
                code: self.code(),
                message,
            },
        };

        (status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping_is_stable() {
        assert_eq!(AppError::Unauthorized.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            AppError::BadRequest("x".into()).status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            AppError::NotImplemented("asr".into()).status(),
            StatusCode::NOT_IMPLEMENTED
        );
        assert_eq!(
            AppError::Internal(anyhow::anyhow!("boom")).status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn internal_errors_do_not_leak_details() {
        let err = AppError::Internal(anyhow::anyhow!("secret path C:\\keys\\prod.pem"));
        assert!(!err.to_string().contains("secret"));
    }

    #[test]
    fn codes_are_snake_case() {
        let all = [
            AppError::BadRequest(String::new()),
            AppError::Unauthorized,
            AppError::NotFound(String::new()),
            AppError::PayloadTooLarge { limit: 1 },
            AppError::NotImplemented(String::new()),
            AppError::Upstream(String::new()),
            AppError::Internal(anyhow::anyhow!("x")),
        ];
        for e in all {
            let c = e.code();
            assert!(
                c.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_'),
                "错误码 {c:?} 不是 snake_case"
            );
        }
    }
}
