use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use thiserror::Error;

use crate::{jwt::JwtError, store::StoreError, wechat::WeChatError};

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("bad request: {0}")]
    BadRequest(String),
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

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
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
