use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use thiserror::Error;

use crate::{jwt::JwtError, store::StoreError, wechat::WeChatError};

// 这个文件解决一个问题：不同模块会产生不同错误，HTTP API 最后必须统一返回状态码和 JSON

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("bad request: {0}")]
    BadRequest(String), // 带括号的 variant 可以携带数据，比如 BadRequest(String) 会带一段错误信息。
    #[error("unauthorized")]
    Unauthorized,
    #[error("not found")]
    NotFound,
    #[error("account conflict")]
    AccountConflict,
    #[error("upstream request failed: {0}")]
    Upstream(String),
    #[error("internal error: {0}")]
    Internal(String),
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: &'static str,
    message: String,
}

// 把 ApiError 变成 HTTP 响应
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        // match 类似其他语言里的 switch，但 Rust 要求覆盖所有情况。
        let (status, code, message) = match self {
            ApiError::BadRequest(message) => (StatusCode::BAD_REQUEST, "bad_request", message),
            ApiError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "missing or invalid bearer token".to_string(),
            ),
            ApiError::NotFound => (
                StatusCode::NOT_FOUND,
                "not_found",
                "no route matched this request".to_string(),
            ),
            ApiError::AccountConflict => (
                StatusCode::CONFLICT,
                "account_conflict",
                "openid and phone number belong to different users".to_string(),
            ),
            ApiError::Upstream(message) => (StatusCode::BAD_GATEWAY, "bad_gateway", message),
            ApiError::Internal(message) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
            }
        };

        (
            status,
            Json(ErrorBody {
                error: code,
                message,
            }),
        )
            .into_response()
    }
}

/// 为什么 handler 里的 ? 能自动变成 ApiError
//
// impl From<StoreError> for ApiError
// impl From<JwtError> for ApiError
// impl From<WeChatError> for ApiError
//
// 这些实现定义了错误转换规则。

impl From<StoreError> for ApiError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::AccountConflict => ApiError::AccountConflict,
            StoreError::NotFound => ApiError::Unauthorized,
            StoreError::Database(error) => ApiError::Internal(error.to_string()),
        }
    }
}

impl From<JwtError> for ApiError {
    fn from(_: JwtError) -> Self {
        ApiError::Unauthorized
    }
}

impl From<WeChatError> for ApiError {
    fn from(error: WeChatError) -> Self {
        match error {
            WeChatError::Api { code, message } => {
                ApiError::BadRequest(format!("wechat_api_{code}: {message}"))
            }
            WeChatError::InvalidWatermark { .. }
            | WeChatError::MissingOpenid
            | WeChatError::MissingSessionKey
            | WeChatError::MissingAccessToken
            | WeChatError::MissingPhoneInfo
            | WeChatError::InvalidBaseUrl(_)
            | WeChatError::Http(_) => ApiError::Upstream(error.to_string()),
        }
    }
}
