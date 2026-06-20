use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{header, HeaderMap, HeaderName, HeaderValue, Request, Response, StatusCode},
};

use crate::{app::AppState, error::ApiError, models::User};

const MAX_PROXY_BODY_BYTES: usize = 10 * 1024 * 1024;

pub async fn proxy_request(
    State(state): State<AppState>,
    request: Request<Body>,
) -> Result<Response<Body>, ApiError> {
    let (parts, body) = request.into_parts();
    let user = state.authenticated_user(&parts.headers).await?; // 通常 1ms - 5ms 以内，速度很快，不影响性能
    let path = parts.uri.path();
    // More-specific prefixes win so `/api/v1/orders` can override `/api/v1`.
    let upstream = state
        .upstreams
        .iter()
        .filter(|upstream| path_matches_prefix(path, &upstream.prefix))
        .max_by_key(|upstream| upstream.prefix.len())
        .ok_or(ApiError::NotFound)?;

    let path_and_query = parts
        .uri
        .path_and_query()
        .map(|path| path.as_str())
        .unwrap_or(path);
    let url = format!(
        "{}{}",
        upstream.base_url.trim_end_matches('/'),
        path_and_query
    );
    let body = to_bytes(body, MAX_PROXY_BODY_BYTES)
        .await
        .map_err(|error| ApiError::Upstream(error.to_string()))?;

    // 创建发往 upstream 的请求
    let mut request_builder = state.http.request(parts.method, url);
    let forwarded_headers = sanitized_request_headers(&parts.headers, &user);
    for (name, value) in &forwarded_headers {
        request_builder = request_builder.header(name, value);
    }

    let upstream_response = request_builder
        .body(body)
        .send()
        .await
        .map_err(|error| ApiError::Upstream(error.to_string()))?;

    let status = StatusCode::from_u16(upstream_response.status().as_u16())
        .map_err(|error| ApiError::Upstream(error.to_string()))?;
    let upstream_headers = upstream_response.headers().clone();
    let bytes = upstream_response
        .bytes()
        .await
        .map_err(|error| ApiError::Upstream(error.to_string()))?;

    let mut response = Response::builder()
        .status(status)
        .body(Body::from(bytes))
        .map_err(|error| ApiError::Upstream(error.to_string()))?;
    for (name, value) in upstream_headers {
        if let Some(name) = name {
            if should_forward_response_header(&name) {
                response.headers_mut().append(name, value);
            }
        }
    }

    Ok(response)
}

pub fn sanitized_request_headers(source: &HeaderMap, user: &User) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in source {
        if should_forward_request_header(name) {
            headers.append(name, value.clone());
        }
    }

    // Identity headers are generated from the verified gateway user only. Any
    // client-supplied x-user-* or x-wechat-* headers are stripped below.
    headers.insert(
        HeaderName::from_static("x-gateway-authenticated"),
        HeaderValue::from_static("true"),
    );
    headers.insert(
        HeaderName::from_static("x-user-id"),
        HeaderValue::from_str(&user.id.to_string()).expect("UUID is a valid header value"),
    );
    headers.insert(
        HeaderName::from_static("x-openid-bound"),
        HeaderValue::from_static(if user.openid_bound() { "true" } else { "false" }),
    );
    headers.insert(
        HeaderName::from_static("x-phone-verified"),
        HeaderValue::from_static(if user.phone_verified() {
            "true"
        } else {
            "false"
        }),
    );

    headers
}

fn should_forward_request_header(name: &HeaderName) -> bool {
    // Never let callers smuggle identity or connection-management headers to
    // downstream services through the gateway.
    !is_hop_by_hop_header(name)
        && name != header::HOST
        && name != header::AUTHORIZATION
        && name != header::CONTENT_LENGTH
        && name != HeaderName::from_static("x-gateway-authenticated")
        && !name.as_str().starts_with("x-user-")
        && !name.as_str().starts_with("x-wechat-")
}

fn should_forward_response_header(name: &HeaderName) -> bool {
    !is_hop_by_hop_header(name) && name != header::CONTENT_LENGTH
}

fn is_hop_by_hop_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    if prefix == "/" {
        return true;
    }
    let prefix = prefix.trim_end_matches('/');
    path == prefix || path.starts_with(&format!("{prefix}/"))
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use uuid::Uuid;

    use super::*;

    #[test]
    fn strips_spoofed_identity_headers_and_injects_context() {
        let mut source = HeaderMap::new();
        source.insert("x-user-id", HeaderValue::from_static("spoofed"));
        source.insert("x-user-role", HeaderValue::from_static("admin"));
        source.insert("x-wechat-openid", HeaderValue::from_static("spoofed"));
        source.insert("authorization", HeaderValue::from_static("Bearer token"));
        source.insert("x-request-id", HeaderValue::from_static("request-1"));

        let user = User {
            id: Uuid::new_v4(),
            openid: Some("openid-1".to_string()),
            unionid: None,
            country_code: Some("86".to_string()),
            pure_phone_number: Some("13800138000".to_string()),
            phone_number: Some("+8613800138000".to_string()),
            phone_verified_at: Some(Utc::now()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let sanitized = sanitized_request_headers(&source, &user);

        assert_eq!(
            sanitized.get("x-request-id"),
            Some(&HeaderValue::from_static("request-1"))
        );
        assert_eq!(
            sanitized.get("x-gateway-authenticated"),
            Some(&HeaderValue::from_static("true"))
        );
        assert_eq!(
            sanitized.get("x-user-id"),
            Some(&HeaderValue::from_str(&user.id.to_string()).unwrap())
        );
        assert_eq!(
            sanitized.get("x-openid-bound"),
            Some(&HeaderValue::from_static("true"))
        );
        assert_eq!(
            sanitized.get("x-phone-verified"),
            Some(&HeaderValue::from_static("true"))
        );
        assert!(sanitized.get("authorization").is_none());
        assert!(sanitized.get("x-user-role").is_none());
        assert!(sanitized.get("x-wechat-openid").is_none());
    }

    #[test]
    fn matches_prefix_on_path_boundary() {
        assert!(path_matches_prefix("/api/v1/orders", "/api/v1"));
        assert!(path_matches_prefix("/api/v1", "/api/v1"));
        assert!(!path_matches_prefix("/api/v10/orders", "/api/v1"));
    }
}
