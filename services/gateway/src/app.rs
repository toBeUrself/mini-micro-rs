use std::sync::Arc;

use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    config::UpstreamConfig,
    error::ApiError,
    jwt::JwtManager,
    models::{User, UserResponse},
    proxy::proxy_request,
    store::UserStore,
    wechat::WeChatClient,
};

/// Shared runtime dependencies for every route handler.
///
/// The concrete storage implementation is hidden behind `UserStore` so handler
/// tests can use an in-memory store while production uses Postgres.
// AppState：所有 handler 共享的依赖
// pub 完全公开，其他crate也可以访问，pub(crate)只限当前crate访问
#[derive(Clone)]
pub struct AppState {
    pub(crate) store: Arc<dyn UserStore>,
    pub(crate) wechat: WeChatClient,
    pub(crate) jwt: JwtManager,
    pub(crate) upstreams: Arc<Vec<UpstreamConfig>>,
    pub(crate) http: reqwest::Client,
}

// 请求和响应 struct
// 这表示请求 JSON 长这样：

//  {
//    "code": "xxx"
//  }
// Deserialize 的意思是：可以从 JSON 反序列化成 Rust struct。
#[derive(Debug, Deserialize)]
struct WeChatLoginRequest {
    code: String,
}

#[derive(Debug, Deserialize)]
struct WeChatPhoneLoginRequest {
    login_code: String,
    phone_code: String,
}

#[derive(Debug, Deserialize)]
struct BindPhoneRequest {
    code: String,
}

// Serialize 的意思是：可以从 Rust struct 序列化成 JSON。
#[derive(Debug, Serialize)]
struct AuthResponse {
    token_type: &'static str, // &'static str 先简单理解成“程序整个生命周期都有效的字符串字面量”，这里就是固定返回 "Bearer"。
    access_token: String,
    expires_in: i64,
    user: UserResponse,
}

impl AppState {
    pub fn new(
        store: Arc<dyn UserStore>,
        wechat: WeChatClient,
        jwt: JwtManager,
        upstreams: Vec<UpstreamConfig>,
    ) -> Self {
        Self {
            store,
            wechat,
            jwt,
            upstreams: Arc::new(upstreams),
            http: reqwest::Client::new(),
        }
    }

    /// Validate a business JWT and load the current user from storage.
    ///
    /// Proxy routes intentionally reload the user on each request so downstream
    /// identity headers reflect the current binding state instead of stale JWT
    /// claims.
    pub(crate) async fn authenticated_user(&self, headers: &HeaderMap) -> Result<User, ApiError> {
        let token = bearer_token(headers)?;
        let user_id = self.jwt.verify(token)?;
        self.store
            .get_by_id(user_id)
            .await?
            .ok_or(ApiError::Unauthorized)
    }
}

// router(state)：注册路由
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz)) // GET /healthz 调 healthz
        .route("/api/v1/auth/wechat-login", post(wechat_login)) // POST /api/v1/auth/wechat-login 调 wechat_login
        .route("/api/v1/auth/wechat-phone-login", post(wechat_phone_login))
        .route("/api/v1/auth/bind-phone", post(bind_phone))
        .fallback(proxy_request) // 其他没匹配上的请求走 proxy_request
        .with_state(state) // 把共享依赖挂到整个 Router 上
}

// 最简单 handler：healthz
async fn healthz() -> StatusCode {
    StatusCode::OK // 它没有请求体，也不需要 state，直接返回 200 OK。
}

// 微信登录 handler
async fn wechat_login(
    State(state): State<AppState>, // Json<WeChatLoginRequest> 表示：告诉 axum：这个参数要从 HTTP 请求体里按 JSON 格式提取出来。
    Json(request): Json<WeChatLoginRequest>, // #[derive(Deserialize)] 表示：这个 struct 具备“从 JSON 数据反序列化成 Rust 对象”的能力。
) -> Result<Json<AuthResponse>, ApiError> {
    ensure_non_empty(&request.code, "code")?;

    let session = state.wechat.code2_session(&request.code).await?;
    let user = state
        .store
        .find_or_create_by_wechat(&session.openid, session.unionid.as_deref())
        .await?;

    issue_auth_response(&state, user)
}

async fn wechat_phone_login(
    State(state): State<AppState>,
    Json(request): Json<WeChatPhoneLoginRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    ensure_non_empty(&request.login_code, "login_code")?;
    ensure_non_empty(&request.phone_code, "phone_code")?;

    // The WeChat login code and phone code are different credentials. They are
    // exchanged independently and then reconciled inside the store transaction.
    let session = state.wechat.code2_session(&request.login_code).await?;
    let phone = state.wechat.get_phone_number(&request.phone_code).await?;
    let user = state
        .store
        .find_or_create_by_wechat_and_bind_phone(
            &session.openid,
            session.unionid.as_deref(),
            &phone,
        )
        .await?;

    issue_auth_response(&state, user)
}

async fn bind_phone(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<BindPhoneRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    ensure_non_empty(&request.code, "code")?;

    let user = state.authenticated_user(&headers).await?;
    let phone = state.wechat.get_phone_number(&request.code).await?;
    let user = state.store.bind_phone(user.id, &phone).await?;

    issue_auth_response(&state, user)
}

// 小工具函数：统一签发登录响应
fn issue_auth_response(state: &AppState, user: User) -> Result<Json<AuthResponse>, ApiError> {
    let issued = state.jwt.issue(user.id)?;
    Ok(Json(AuthResponse {
        token_type: "Bearer",
        access_token: issued.token,
        expires_in: issued.expires_in,
        user: UserResponse::from(&user),
    }))
}

fn bearer_token(headers: &HeaderMap) -> Result<&str, ApiError> {
    let value = headers
        .get(header::AUTHORIZATION)
        .ok_or(ApiError::Unauthorized)?
        .to_str()
        .map_err(|_| ApiError::Unauthorized)?;

    // Only accept the exact Bearer scheme used by the gateway-issued token.
    value
        .strip_prefix("Bearer ")
        .filter(|token| !token.is_empty())
        .ok_or(ApiError::Unauthorized)
}

fn ensure_non_empty(value: &str, field: &'static str) -> Result<(), ApiError> {
    if value.trim().is_empty() {
        return Err(ApiError::BadRequest(format!("{field} is required")));
    }
    Ok(())
}

#[allow(dead_code)]
fn _assert_uuid_send_sync(_: Uuid) {}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use async_trait::async_trait;
    use axum::{
        body::{to_bytes, Body},
        extract::{Query, State},
        http::{header, HeaderMap, Request, StatusCode},
        response::IntoResponse,
        routing::{get, post},
        Json, Router,
    };
    use chrono::Utc;
    use serde_json::{json, Value};
    use tokio::{net::TcpListener, sync::Mutex};
    use tower::ServiceExt;

    use super::*;
    use crate::{
        models::VerifiedPhoneNumber,
        store::{StoreError, UserStore},
    };

    #[tokio::test]
    async fn healthz_returns_ok() {
        let app = test_app().await.0;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn wechat_login_returns_jwt() {
        let (app, store, jwt) = test_app().await;

        let response = app
            .oneshot(json_request(
                "/api/v1/auth/wechat-login",
                json!({ "code": "login-code" }),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        let token = body["access_token"].as_str().expect("token should exist");
        let user_id = jwt.verify(token).expect("token should verify");
        let user = store.get_by_id(user_id).await.unwrap().unwrap();

        assert_eq!(user.openid.as_deref(), Some("openid-login-code"));
        assert_eq!(body["user"]["openid_bound"], true);
    }

    #[tokio::test]
    async fn wechat_phone_login_binds_openid_and_phone() {
        let (app, store, jwt) = test_app().await;

        let response = app
            .oneshot(json_request(
                "/api/v1/auth/wechat-phone-login",
                json!({
                    "login_code": "combo-code",
                    "phone_code": "phone-code"
                }),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        let token = body["access_token"].as_str().expect("token should exist");
        let user_id = jwt.verify(token).expect("token should verify");
        let user = store.get_by_id(user_id).await.unwrap().unwrap();

        assert_eq!(user.openid.as_deref(), Some("openid-combo-code"));
        assert_eq!(user.phone_number.as_deref(), Some("+8613800138000"));
        assert!(user.phone_verified());
    }

    #[tokio::test]
    async fn bind_phone_requires_jwt() {
        let (app, _, _) = test_app().await;

        let response = app
            .oneshot(json_request(
                "/api/v1/auth/bind-phone",
                json!({ "code": "phone-code" }),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn phone_conflict_returns_409() {
        let (app, store, _) = test_app().await;
        let phone = VerifiedPhoneNumber {
            country_code: "86".to_string(),
            pure_phone_number: "13800138000".to_string(),
            phone_number: "+8613800138000".to_string(),
        };
        store
            .find_or_create_by_wechat_and_bind_phone("openid-existing", None, &phone)
            .await
            .unwrap();

        let response = app
            .oneshot(json_request(
                "/api/v1/auth/wechat-phone-login",
                json!({
                    "login_code": "new-openid",
                    "phone_code": "phone-code"
                }),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = response_json(response).await;
        assert_eq!(body["error"], "account_conflict");
    }

    #[tokio::test]
    async fn protected_route_requires_jwt() {
        let (app, _, _) = test_app_with_upstream().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/orders")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn protected_route_proxies_with_gateway_user_headers() {
        let (app, store, jwt) = test_app_with_upstream().await;
        let user = store
            .find_or_create_by_wechat("openid-proxy", None)
            .await
            .unwrap();
        let token = jwt.issue(user.id).unwrap().token;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/orders")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("x-user-role", "admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;

        assert_eq!(body["x-gateway-authenticated"], "true");
        assert_eq!(body["x-user-id"], user.id.to_string());
        assert!(body.get("x-user-role").is_none());
    }

    #[tokio::test]
    async fn public_route_skips_auth() {
        let (app, _, _) = test_app_with_upstream().await;

        // 路径中包含 /public/ 段的公开 API 无需 JWT 即可访问。
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/public/market/klines")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        // 公开路径不注入用户身份 header。
        assert!(body.get("x-gateway-authenticated").is_none());
        assert!(body.get("x-user-id").is_none());
    }

    #[tokio::test]
    async fn non_public_route_still_requires_jwt() {
        let (app, _, _) = test_app_with_upstream().await;

        // 路径中不含 /public/ 段，仍需要 JWT。
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/orders")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    async fn test_app() -> (Router, Arc<MemoryUserStore>, JwtManager) {
        let wechat_api_base = spawn_wechat_mock().await;
        build_test_app(vec![], wechat_api_base)
    }

    async fn test_app_with_upstream() -> (Router, Arc<MemoryUserStore>, JwtManager) {
        let wechat_api_base = spawn_wechat_mock().await;
        let upstream_base = spawn_upstream_mock().await;
        build_test_app(
            vec![UpstreamConfig {
                prefix: "/api/v1".to_string(),
                base_url: upstream_base,
            }],
            wechat_api_base,
        )
    }

    fn build_test_app(
        upstreams: Vec<UpstreamConfig>,
        wechat_api_base: String,
    ) -> (Router, Arc<MemoryUserStore>, JwtManager) {
        let store = Arc::new(MemoryUserStore::default());
        let jwt = JwtManager::new("test-secret", 3600);
        let state = AppState::new(
            store.clone(),
            WeChatClient::new("wx-test", "wechat-secret", wechat_api_base),
            jwt.clone(),
            upstreams,
        );
        (router(state), store, jwt)
    }

    async fn spawn_wechat_mock() -> String {
        let app = Router::new()
            .route("/sns/jscode2session", get(mock_code2_session))
            .route("/cgi-bin/token", get(mock_access_token))
            .route("/wxa/business/getuserphonenumber", post(mock_phone_number))
            .with_state("wx-test".to_string());
        spawn_router(app).await
    }

    async fn spawn_upstream_mock() -> String {
        let app = Router::new().fallback(mock_upstream);
        spawn_router(app).await
    }

    async fn spawn_router(app: Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{address}")
    }

    async fn mock_code2_session(Query(params): Query<HashMap<String, String>>) -> Json<Value> {
        let code = params.get("js_code").cloned().unwrap_or_default();
        Json(json!({
            "openid": format!("openid-{code}"),
            "session_key": "session-key",
            "unionid": format!("union-{code}")
        }))
    }

    async fn mock_access_token() -> Json<Value> {
        Json(json!({
            "access_token": "mock-access-token",
            "expires_in": 7200
        }))
    }

    async fn mock_phone_number(State(app_id): State<String>) -> Json<Value> {
        Json(json!({
            "errcode": 0,
            "phone_info": {
                "phoneNumber": "+8613800138000",
                "purePhoneNumber": "13800138000",
                "countryCode": "86",
                "watermark": {
                    "timestamp": 1700000000,
                    "appid": app_id
                }
            }
        }))
    }

    async fn mock_upstream(headers: HeaderMap) -> impl IntoResponse {
        let mut body = serde_json::Map::new();
        for (name, value) in headers {
            if let Some(name) = name {
                if let Ok(value) = value.to_str() {
                    body.insert(name.to_string(), Value::String(value.to_string()));
                }
            }
        }
        Json(Value::Object(body))
    }

    fn json_request(uri: &str, body: Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    async fn response_json(response: axum::response::Response) -> Value {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[derive(Default)]
    struct MemoryUserStore {
        users: Mutex<Vec<User>>,
    }

    #[async_trait]
    impl UserStore for MemoryUserStore {
        async fn find_or_create_by_wechat(
            &self,
            openid: &str,
            unionid: Option<&str>,
        ) -> Result<User, StoreError> {
            let mut users = self.users.lock().await;
            if let Some(user) = users
                .iter_mut()
                .find(|user| user.openid.as_deref() == Some(openid))
            {
                if user.unionid.is_none() {
                    user.unionid = unionid.map(ToOwned::to_owned);
                }
                return Ok(user.clone());
            }

            let user = new_user(
                Some(openid.to_string()),
                unionid.map(ToOwned::to_owned),
                None,
            );
            users.push(user.clone());
            Ok(user)
        }

        async fn find_or_create_by_wechat_and_bind_phone(
            &self,
            openid: &str,
            unionid: Option<&str>,
            phone: &VerifiedPhoneNumber,
        ) -> Result<User, StoreError> {
            let mut users = self.users.lock().await;
            let openid_index = users
                .iter()
                .position(|user| user.openid.as_deref() == Some(openid));
            let phone_index = users.iter().position(|user| phone_matches(user, phone));

            if let (Some(openid_index), Some(phone_index)) = (openid_index, phone_index) {
                if openid_index != phone_index {
                    return Err(StoreError::AccountConflict);
                }
            }

            let index = match (openid_index, phone_index) {
                (Some(index), _) => index,
                (None, Some(index)) => {
                    if users[index].openid.is_some() {
                        return Err(StoreError::AccountConflict);
                    }
                    users[index].openid = Some(openid.to_string());
                    users[index].unionid = users[index]
                        .unionid
                        .clone()
                        .or_else(|| unionid.map(ToOwned::to_owned));
                    index
                }
                (None, None) => {
                    let user = new_user(
                        Some(openid.to_string()),
                        unionid.map(ToOwned::to_owned),
                        Some(phone),
                    );
                    users.push(user);
                    users.len() - 1
                }
            };

            apply_phone(&mut users[index], phone);
            Ok(users[index].clone())
        }

        async fn bind_phone(
            &self,
            user_id: Uuid,
            phone: &VerifiedPhoneNumber,
        ) -> Result<User, StoreError> {
            let mut users = self.users.lock().await;
            let index = users
                .iter()
                .position(|user| user.id == user_id)
                .ok_or(StoreError::NotFound)?;
            if users
                .iter()
                .any(|user| user.id != user_id && phone_matches(user, phone))
            {
                return Err(StoreError::AccountConflict);
            }

            apply_phone(&mut users[index], phone);
            Ok(users[index].clone())
        }

        async fn get_by_id(&self, user_id: Uuid) -> Result<Option<User>, StoreError> {
            let users = self.users.lock().await;
            Ok(users.iter().find(|user| user.id == user_id).cloned())
        }
    }

    fn new_user(
        openid: Option<String>,
        unionid: Option<String>,
        phone: Option<&VerifiedPhoneNumber>,
    ) -> User {
        let now = Utc::now();
        let mut user = User {
            id: Uuid::new_v4(),
            openid,
            unionid,
            country_code: None,
            pure_phone_number: None,
            phone_number: None,
            phone_verified_at: None,
            created_at: now,
            updated_at: now,
        };
        if let Some(phone) = phone {
            apply_phone(&mut user, phone);
        }
        user
    }

    fn apply_phone(user: &mut User, phone: &VerifiedPhoneNumber) {
        user.country_code = Some(phone.country_code.clone());
        user.pure_phone_number = Some(phone.pure_phone_number.clone());
        user.phone_number = Some(phone.phone_number.clone());
        user.phone_verified_at = Some(Utc::now());
        user.updated_at = Utc::now();
    }

    fn phone_matches(user: &User, phone: &VerifiedPhoneNumber) -> bool {
        user.country_code.as_deref() == Some(phone.country_code.as_str())
            && user.pure_phone_number.as_deref() == Some(phone.pure_phone_number.as_str())
    }
}
