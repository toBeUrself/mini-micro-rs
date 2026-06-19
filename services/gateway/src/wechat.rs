use std::{sync::Arc, time::Duration};

use reqwest::Url;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::models::VerifiedPhoneNumber;

#[derive(Clone)]
pub struct WeChatClient {
    inner: Arc<WeChatClientInner>,
}

struct WeChatClientInner {
    http: reqwest::Client,
    app_id: String,
    app_secret: String,
    api_base: String,
    // First version is single-instance, so process-local caching is enough.
    // Multi-instance deployments should move this to Redis or another shared
    // coordination layer before relying on global token reuse.
    access_token: Mutex<Option<CachedAccessToken>>,
}

#[derive(Debug, Clone)]
struct CachedAccessToken {
    token: String,
    expires_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeChatSession {
    pub openid: String,
    pub unionid: Option<String>,
    pub session_key: String,
}

#[derive(Debug, Error)]
pub enum WeChatError {
    #[error("invalid WeChat API base URL: {0}")]
    InvalidBaseUrl(#[source] url::ParseError),
    #[error("WeChat HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("WeChat API returned errcode {code}: {message}")]
    Api { code: i64, message: String },
    #[error("WeChat response did not include openid")]
    MissingOpenid,
    #[error("WeChat response did not include session_key")]
    MissingSessionKey,
    #[error("WeChat response did not include access_token")]
    MissingAccessToken,
    #[error("WeChat response did not include phone_info")]
    MissingPhoneInfo,
    #[error("phone watermark appid mismatch: expected {expected}, got {actual}")]
    InvalidWatermark { expected: String, actual: String },
}

#[derive(Debug, Deserialize)]
pub struct Code2SessionResponse {
    pub openid: Option<String>,
    pub session_key: Option<String>,
    pub unionid: Option<String>,
    pub errcode: Option<i64>,
    pub errmsg: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AccessTokenResponse {
    access_token: Option<String>,
    expires_in: Option<u64>,
    errcode: Option<i64>,
    errmsg: Option<String>,
}

#[derive(Debug, Serialize)]
struct PhoneNumberRequest<'a> {
    code: &'a str,
}

#[derive(Debug, Deserialize)]
pub struct PhoneNumberResponse {
    pub errcode: Option<i64>,
    pub errmsg: Option<String>,
    pub phone_info: Option<PhoneInfoPayload>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneInfoPayload {
    pub phone_number: String,
    pub pure_phone_number: String,
    pub country_code: String,
    pub watermark: PhoneWatermark,
}

#[derive(Debug, Deserialize)]
pub struct PhoneWatermark {
    pub appid: String,
    pub timestamp: Option<i64>,
}

impl WeChatClient {
    pub fn new(
        app_id: impl Into<String>,
        app_secret: impl Into<String>,
        api_base: impl Into<String>,
    ) -> Self {
        Self {
            inner: Arc::new(WeChatClientInner {
                http: reqwest::Client::new(),
                app_id: app_id.into(),
                app_secret: app_secret.into(),
                api_base: api_base.into().trim_end_matches('/').to_string(),
                access_token: Mutex::new(None),
            }),
        }
    }

    pub async fn code2_session(&self, code: &str) -> Result<WeChatSession, WeChatError> {
        let url = self.endpoint("/sns/jscode2session")?;
        let response = self
            .inner
            .http
            .get(url)
            .query(&[
                ("appid", self.inner.app_id.as_str()),
                ("secret", self.inner.app_secret.as_str()),
                ("js_code", code),
                ("grant_type", "authorization_code"),
            ])
            .send()
            .await?
            .error_for_status()?
            .json::<Code2SessionResponse>()
            .await?;

        response.into_session()
    }

    pub async fn get_phone_number(&self, code: &str) -> Result<VerifiedPhoneNumber, WeChatError> {
        let access_token = self.access_token().await?;
        let mut url = self.endpoint("/wxa/business/getuserphonenumber")?;
        url.query_pairs_mut()
            .append_pair("access_token", &access_token);

        let response = self
            .inner
            .http
            .post(url)
            .json(&PhoneNumberRequest { code })
            .send()
            .await?
            .error_for_status()?
            .json::<PhoneNumberResponse>()
            .await?;

        response.into_verified_phone(&self.inner.app_id)
    }

    fn endpoint(&self, path: &str) -> Result<Url, WeChatError> {
        let base = Url::parse(&self.inner.api_base).map_err(WeChatError::InvalidBaseUrl)?;
        base.join(path.trim_start_matches('/'))
            .map_err(WeChatError::InvalidBaseUrl)
    }

    async fn access_token(&self) -> Result<String, WeChatError> {
        let mut guard = self.inner.access_token.lock().await;
        if let Some(cached) = guard.as_ref() {
            if cached.expires_at > Instant::now() {
                return Ok(cached.token.clone());
            }
        }

        let url = self.endpoint("/cgi-bin/token")?;
        let response = self
            .inner
            .http
            .get(url)
            .query(&[
                ("grant_type", "client_credential"),
                ("appid", self.inner.app_id.as_str()),
                ("secret", self.inner.app_secret.as_str()),
            ])
            .send()
            .await?
            .error_for_status()?
            .json::<AccessTokenResponse>()
            .await?;

        response.ensure_success()?;
        let token = response
            .access_token
            .ok_or(WeChatError::MissingAccessToken)?;
        let ttl = response.expires_in.unwrap_or(7200);
        // Refresh slightly before WeChat's advertised expiry to avoid using a
        // token that expires while an in-flight getPhoneNumber request runs.
        let cache_ttl = ttl.saturating_sub(60).max(1);
        *guard = Some(CachedAccessToken {
            token: token.clone(),
            expires_at: Instant::now() + Duration::from_secs(cache_ttl),
        });

        Ok(token)
    }
}

impl Code2SessionResponse {
    pub fn into_session(self) -> Result<WeChatSession, WeChatError> {
        self.ensure_success()?;
        Ok(WeChatSession {
            openid: self.openid.ok_or(WeChatError::MissingOpenid)?,
            unionid: self.unionid,
            session_key: self.session_key.ok_or(WeChatError::MissingSessionKey)?,
        })
    }

    fn ensure_success(&self) -> Result<(), WeChatError> {
        if let Some(code) = self.errcode {
            if code != 0 {
                return Err(WeChatError::Api {
                    code,
                    message: self.errmsg.clone().unwrap_or_default(),
                });
            }
        }
        Ok(())
    }
}

impl AccessTokenResponse {
    fn ensure_success(&self) -> Result<(), WeChatError> {
        if let Some(code) = self.errcode {
            if code != 0 {
                return Err(WeChatError::Api {
                    code,
                    message: self.errmsg.clone().unwrap_or_default(),
                });
            }
        }
        Ok(())
    }
}

impl PhoneNumberResponse {
    pub fn into_verified_phone(
        self,
        expected_app_id: &str,
    ) -> Result<VerifiedPhoneNumber, WeChatError> {
        self.ensure_success()?;
        let phone_info = self.phone_info.ok_or(WeChatError::MissingPhoneInfo)?;
        // The phone code must belong to the same mini-program as the configured
        // gateway. Without this check, a code from another app could bind PII to
        // the wrong business account.
        if phone_info.watermark.appid != expected_app_id {
            return Err(WeChatError::InvalidWatermark {
                expected: expected_app_id.to_string(),
                actual: phone_info.watermark.appid,
            });
        }

        Ok(VerifiedPhoneNumber {
            country_code: phone_info.country_code,
            pure_phone_number: phone_info.pure_phone_number,
            phone_number: phone_info.phone_number,
        })
    }

    fn ensure_success(&self) -> Result<(), WeChatError> {
        if let Some(code) = self.errcode {
            if code != 0 {
                return Err(WeChatError::Api {
                    code,
                    message: self.errmsg.clone().unwrap_or_default(),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_code2_session_response() {
        let response: Code2SessionResponse = serde_json::from_str(
            r#"{
                "openid": "openid-1",
                "session_key": "session-key",
                "unionid": "union-1"
            }"#,
        )
        .expect("response should parse");

        let session = response.into_session().expect("session should convert");

        assert_eq!(
            session,
            WeChatSession {
                openid: "openid-1".to_string(),
                unionid: Some("union-1".to_string()),
                session_key: "session-key".to_string(),
            }
        );
    }

    #[test]
    fn parses_phone_response_and_checks_watermark() {
        let response: PhoneNumberResponse = serde_json::from_str(
            r#"{
                "errcode": 0,
                "phone_info": {
                    "phoneNumber": "+8613800138000",
                    "purePhoneNumber": "13800138000",
                    "countryCode": "86",
                    "watermark": {
                        "timestamp": 1700000000,
                        "appid": "wx-test"
                    }
                }
            }"#,
        )
        .expect("response should parse");

        let phone = response
            .into_verified_phone("wx-test")
            .expect("watermark should match");

        assert_eq!(
            phone,
            VerifiedPhoneNumber {
                country_code: "86".to_string(),
                pure_phone_number: "13800138000".to_string(),
                phone_number: "+8613800138000".to_string(),
            }
        );
    }

    #[test]
    fn rejects_phone_response_for_wrong_appid() {
        let response: PhoneNumberResponse = serde_json::from_str(
            r#"{
                "errcode": 0,
                "phone_info": {
                    "phoneNumber": "+8613800138000",
                    "purePhoneNumber": "13800138000",
                    "countryCode": "86",
                    "watermark": {
                        "appid": "wx-other"
                    }
                }
            }"#,
        )
        .expect("response should parse");

        let error = response
            .into_verified_phone("wx-test")
            .expect_err("wrong appid should fail");

        assert!(matches!(error, WeChatError::InvalidWatermark { .. }));
    }
}
