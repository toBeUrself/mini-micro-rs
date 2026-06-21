use async_trait::async_trait;
use sqlx::{postgres::PgPoolOptions, PgPool};
use thiserror::Error;
use uuid::Uuid;

use crate::models::{User, VerifiedPhoneNumber};

// 定义“用户存储”应该有哪些能力
#[async_trait]
pub trait UserStore: Send + Sync {
    async fn find_or_create_by_wechat(
        &self,
        openid: &str,
        unionid: Option<&str>,
    ) -> Result<User, StoreError>;

    async fn find_or_create_by_wechat_and_bind_phone(
        &self,
        openid: &str,
        unionid: Option<&str>,
        phone: &VerifiedPhoneNumber,
    ) -> Result<User, StoreError>;

    async fn bind_phone(
        &self,
        user_id: Uuid,
        phone: &VerifiedPhoneNumber,
    ) -> Result<User, StoreError>;

    async fn get_by_id(&self, user_id: Uuid) -> Result<Option<User>, StoreError>;
}

// 定义数据库层错误
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("account conflict")]
    AccountConflict,
    #[error("user not found")]
    NotFound,
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

// 真正用 Postgres 实现这些能力
#[derive(Clone)]
pub struct PostgresUserStore {
    pool: PgPool,
}

impl PostgresUserStore {
    pub async fn connect(database_url: &str) -> Result<Self, sqlx::Error> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[async_trait]
impl UserStore for PostgresUserStore {
    // 通过微信 openid 找用户；没有就创建。
    async fn find_or_create_by_wechat(
        &self,
        openid: &str,
        unionid: Option<&str>,
    ) -> Result<User, StoreError> {
        sqlx::query_as::<_, User>(
            r#"
            INSERT INTO users (openid, unionid)
            VALUES ($1, $2)
            ON CONFLICT (openid) WHERE openid IS NOT NULL
            DO UPDATE SET
                unionid = COALESCE(users.unionid, EXCLUDED.unionid),
                updated_at = now()
            RETURNING id, openid, unionid, country_code, pure_phone_number,
                      phone_number, phone_verified_at, created_at, updated_at
            "#,
        )
        .bind(openid) // 把 openid 绑定到 SQL 里的 $1。
        .bind(unionid) // 把 unionid 绑定到 SQL 里的 $2。
        .fetch_one(&self.pool) // 期望数据库返回一行。
        .await // 等待数据库执行完成。
        .map_err(map_sqlx_error) // 如果出错，把 sqlx 错误转换成 StoreError。
    }

    // 通过微信登录，同时绑定手机号。
    async fn find_or_create_by_wechat_and_bind_phone(
        &self,
        openid: &str,
        unionid: Option<&str>,
        phone: &VerifiedPhoneNumber,
    ) -> Result<User, StoreError> {
        let mut tx = self.pool.begin().await?;

        // Lock both possible identities before deciding what to update. This
        // prevents two concurrent login/bind requests from silently merging
        // separate accounts through interleaved reads and writes.
        let openid_user = sqlx::query_as::<_, User>(
            r#"
            SELECT id, openid, unionid, country_code, pure_phone_number,
                   phone_number, phone_verified_at, created_at, updated_at
            FROM users
            WHERE openid = $1
            FOR UPDATE
            "#,
        )
        .bind(openid)
        .fetch_optional(&mut *tx)
        .await?;

        let phone_user = sqlx::query_as::<_, User>(
            r#"
            SELECT id, openid, unionid, country_code, pure_phone_number,
                   phone_number, phone_verified_at, created_at, updated_at
            FROM users
            WHERE country_code = $1 AND pure_phone_number = $2
            FOR UPDATE
            "#,
        )
        .bind(&phone.country_code)
        .bind(&phone.pure_phone_number)
        .fetch_optional(&mut *tx)
        .await?;

        if let (Some(openid_user), Some(phone_user)) = (&openid_user, &phone_user) {
            if openid_user.id != phone_user.id {
                // Account merge is intentionally explicit product work. The
                // gateway returns 409 so a higher-level flow can resolve it.
                return Err(StoreError::AccountConflict);
            }
        }

        let user = match (openid_user, phone_user) {
            (Some(user), _) => update_phone(&mut tx, user.id, phone).await?,
            (None, Some(user)) => {
                if user.openid.is_some() {
                    return Err(StoreError::AccountConflict);
                }
                update_wechat_and_phone(&mut tx, user.id, openid, unionid, phone).await?
            }
            (None, None) => insert_wechat_and_phone(&mut tx, openid, unionid, phone).await?,
        };

        tx.commit().await?;
        Ok(user)
    }

    // 给已有用户绑定手机号。
    async fn bind_phone(
        &self,
        user_id: Uuid,
        phone: &VerifiedPhoneNumber,
    ) -> Result<User, StoreError> {
        let mut tx = self.pool.begin().await?;

        let user = sqlx::query_as::<_, User>(
            r#"
            SELECT id, openid, unionid, country_code, pure_phone_number,
                   phone_number, phone_verified_at, created_at, updated_at
            FROM users
            WHERE id = $1
            FOR UPDATE
            "#,
        )
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(StoreError::NotFound)?;

        let phone_user = sqlx::query_as::<_, User>(
            r#"
            SELECT id, openid, unionid, country_code, pure_phone_number,
                   phone_number, phone_verified_at, created_at, updated_at
            FROM users
            WHERE country_code = $1 AND pure_phone_number = $2
            FOR UPDATE
            "#,
        )
        .bind(&phone.country_code)
        .bind(&phone.pure_phone_number)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(phone_user) = phone_user {
            if phone_user.id != user.id {
                // Binding someone else's verified phone number would be an
                // account takeover path, so fail closed.
                return Err(StoreError::AccountConflict);
            }
        }

        let user = update_phone(&mut tx, user.id, phone).await?;
        tx.commit().await?;
        Ok(user)
    }

    // 通过 user_id 查询用户。
    async fn get_by_id(&self, user_id: Uuid) -> Result<Option<User>, StoreError> {
        sqlx::query_as::<_, User>(
            r#"
            SELECT id, openid, unionid, country_code, pure_phone_number,
                   phone_number, phone_verified_at, created_at, updated_at
            FROM users
            WHERE id = $1
            "#,
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)
    }
}

async fn update_phone(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
    phone: &VerifiedPhoneNumber,
) -> Result<User, StoreError> {
    sqlx::query_as::<_, User>(
        r#"
        UPDATE users
        SET country_code = $2,
            pure_phone_number = $3,
            phone_number = $4,
            phone_verified_at = now(),
            updated_at = now()
        WHERE id = $1
        RETURNING id, openid, unionid, country_code, pure_phone_number,
                  phone_number, phone_verified_at, created_at, updated_at
        "#,
    )
    .bind(user_id)
    .bind(&phone.country_code)
    .bind(&phone.pure_phone_number)
    .bind(&phone.phone_number)
    .fetch_one(&mut **tx)
    .await
    .map_err(map_sqlx_error)
}

async fn update_wechat_and_phone(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
    openid: &str,
    unionid: Option<&str>,
    phone: &VerifiedPhoneNumber,
) -> Result<User, StoreError> {
    sqlx::query_as::<_, User>(
        r#"
        UPDATE users
        SET openid = $2,
            unionid = COALESCE(unionid, $3),
            country_code = $4,
            pure_phone_number = $5,
            phone_number = $6,
            phone_verified_at = now(),
            updated_at = now()
        WHERE id = $1
        RETURNING id, openid, unionid, country_code, pure_phone_number,
                  phone_number, phone_verified_at, created_at, updated_at
        "#,
    )
    .bind(user_id)
    .bind(openid)
    .bind(unionid)
    .bind(&phone.country_code)
    .bind(&phone.pure_phone_number)
    .bind(&phone.phone_number)
    .fetch_one(&mut **tx)
    .await
    .map_err(map_sqlx_error)
}

async fn insert_wechat_and_phone(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    openid: &str,
    unionid: Option<&str>,
    phone: &VerifiedPhoneNumber,
) -> Result<User, StoreError> {
    sqlx::query_as::<_, User>(
        r#"
        INSERT INTO users (
            openid, unionid, country_code, pure_phone_number,
            phone_number, phone_verified_at
        )
        VALUES ($1, $2, $3, $4, $5, now())
        RETURNING id, openid, unionid, country_code, pure_phone_number,
                  phone_number, phone_verified_at, created_at, updated_at
        "#,
    )
    .bind(openid)
    .bind(unionid)
    .bind(&phone.country_code)
    .bind(&phone.pure_phone_number)
    .bind(&phone.phone_number)
    .fetch_one(&mut **tx)
    .await
    .map_err(map_sqlx_error)
}

fn map_sqlx_error(error: sqlx::Error) -> StoreError {
    if let Some(database_error) = error.as_database_error() {
        // Unique indexes are a second line of defense for races that pass the
        // explicit checks above.
        if matches!(
            database_error.constraint(),
            Some("users_openid_unique" | "users_phone_unique")
        ) {
            return StoreError::AccountConflict;
        }
    }
    StoreError::Database(error)
}
