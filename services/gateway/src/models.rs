use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

// Debug：可以调试打印。
// Clone：可以克隆。
// Serialize：可以转成 JSON。
// sqlx::FromRow：可以从 SQL 查询结果行转换成 User。
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub openid: Option<String>,
    pub unionid: Option<String>,
    pub country_code: Option<String>,
    pub pure_phone_number: Option<String>,
    pub phone_number: Option<String>,
    pub phone_verified_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl User {
    // 这个用户是否已经绑定 openid？
    pub fn openid_bound(&self) -> bool {
        self.openid.is_some()
    }

    pub fn phone_verified(&self) -> bool {
        self.phone_verified_at.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedPhoneNumber {
    pub country_code: String,
    pub pure_phone_number: String,
    pub phone_number: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserResponse {
    pub id: Uuid,
    pub openid_bound: bool,
    pub phone_verified: bool,
    pub country_code: Option<String>,
    pub phone_number: Option<String>,
}

impl From<&User> for UserResponse {
    fn from(user: &User) -> Self {
        Self {
            id: user.id,
            openid_bound: user.openid_bound(),
            phone_verified: user.phone_verified(),
            country_code: user.country_code.clone(),
            phone_number: user.phone_number.clone(),
        }
    }
}
