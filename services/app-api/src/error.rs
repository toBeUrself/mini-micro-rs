use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use thiserror::Error;

use crate::store::StoreError;

/// app-api 对外统一返回的错误类型。
///
/// handler 里返回 `Result<T, ApiError>` 后，axum 会调用 `IntoResponse`
/// 把错误变成 HTTP 状态码和 JSON。
#[derive(Debug, Error)]
pub enum ApiError {
    /// 请求参数不合法。
    #[error("bad request: {0}")]
    BadRequest(String),
    /// 服务内部错误。
    #[error("internal error: {0}")]
    Internal(String),
}

/// 错误响应体。
#[derive(Debug, Serialize)]
struct ErrorBody {
    /// 给程序判断用的稳定错误码。
    error: &'static str,
    /// 给开发者看的错误说明。
    message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            ApiError::BadRequest(message) => (StatusCode::BAD_REQUEST, "bad_request", message),
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
        ApiError::Internal(error.to_string())
    }
}
