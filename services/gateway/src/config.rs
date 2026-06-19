use std::{env, fs, net::SocketAddr, path::Path};

use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    pub server: ServerConfig,
    pub wechat: WeChatConfig,
    pub jwt: JwtConfig,
    pub database: DatabaseConfig,
    #[serde(default)]
    pub upstreams: Vec<UpstreamConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub bind: SocketAddr,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WeChatConfig {
    pub app_id: String,
    pub app_secret_env: String,
    #[serde(default = "default_wechat_api_base")]
    pub api_base: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JwtConfig {
    pub secret_env: String,
    #[serde(default = "default_jwt_ttl_seconds")]
    pub ttl_seconds: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub url_env: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct UpstreamConfig {
    pub prefix: String,
    pub base_url: String,
}

#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub bind: SocketAddr,
    pub wechat_app_id: String,
    pub wechat_app_secret: String,
    pub wechat_api_base: String,
    pub jwt_secret: String,
    pub jwt_ttl_seconds: i64,
    pub database_url: String,
    pub upstreams: Vec<UpstreamConfig>,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("required environment variable {name} is not set")]
    MissingEnv { name: String },
    #[error("required environment variable {name} is empty")]
    EmptyEnv { name: String },
    #[error("upstream prefix must start with '/': {0}")]
    InvalidUpstreamPrefix(String),
    #[error("upstream base_url must start with http:// or https://: {0}")]
    InvalidUpstreamBaseUrl(String),
}

impl GatewayConfig {
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path_ref = path.as_ref();
        let content = fs::read_to_string(path_ref).map_err(|source| ConfigError::Io {
            path: path_ref.display().to_string(),
            source,
        })?;
        Self::from_toml_str(&content)
    }

    pub fn from_toml_str(content: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(content)?;
        config.validate()?;
        Ok(config)
    }

    pub fn resolve(self) -> Result<ResolvedConfig, ConfigError> {
        self.validate()?;
        Ok(ResolvedConfig {
            bind: self.server.bind,
            wechat_app_id: self.wechat.app_id,
            // The TOML file stores environment variable names, not secrets.
            // This keeps deploy-specific sensitive values out of source files.
            wechat_app_secret: required_env(&self.wechat.app_secret_env)?,
            wechat_api_base: self.wechat.api_base,
            jwt_secret: required_env(&self.jwt.secret_env)?,
            jwt_ttl_seconds: self.jwt.ttl_seconds,
            database_url: required_env(&self.database.url_env)?,
            upstreams: self.upstreams,
        })
    }

    fn validate(&self) -> Result<(), ConfigError> {
        for upstream in &self.upstreams {
            // Prefixes are matched against request paths in the proxy. Requiring
            // a leading slash prevents accidental broad matches such as "api".
            if !upstream.prefix.starts_with('/') {
                return Err(ConfigError::InvalidUpstreamPrefix(upstream.prefix.clone()));
            }
            if !upstream.base_url.starts_with("http://")
                && !upstream.base_url.starts_with("https://")
            {
                return Err(ConfigError::InvalidUpstreamBaseUrl(
                    upstream.base_url.clone(),
                ));
            }
        }
        Ok(())
    }
}

fn required_env(name: &str) -> Result<String, ConfigError> {
    let value = env::var(name).map_err(|_| ConfigError::MissingEnv {
        name: name.to_string(),
    })?;
    if value.is_empty() {
        return Err(ConfigError::EmptyEnv {
            name: name.to_string(),
        });
    }
    Ok(value)
}

fn default_wechat_api_base() -> String {
    "https://api.weixin.qq.com".to_string()
}

fn default_jwt_ttl_seconds() -> i64 {
    60 * 60 * 24 * 7
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gateway_config() {
        let config = GatewayConfig::from_toml_str(
            r#"
            [server]
            bind = "127.0.0.1:8080"

            [wechat]
            app_id = "wx-test"
            app_secret_env = "WECHAT_APP_SECRET"

            [jwt]
            secret_env = "JWT_SECRET"
            ttl_seconds = 3600

            [database]
            url_env = "DATABASE_URL"

            [[upstreams]]
            prefix = "/api/v1/orders"
            base_url = "http://127.0.0.1:9001"
            "#,
        )
        .expect("config should parse");

        assert_eq!(config.server.bind.to_string(), "127.0.0.1:8080");
        assert_eq!(config.wechat.app_id, "wx-test");
        assert_eq!(config.wechat.api_base, "https://api.weixin.qq.com");
        assert_eq!(config.jwt.ttl_seconds, 3600);
        assert_eq!(
            config.upstreams,
            vec![UpstreamConfig {
                prefix: "/api/v1/orders".to_string(),
                base_url: "http://127.0.0.1:9001".to_string(),
            }]
        );
    }

    #[test]
    fn rejects_invalid_upstream_prefix() {
        let error = GatewayConfig::from_toml_str(
            r#"
            [server]
            bind = "127.0.0.1:8080"

            [wechat]
            app_id = "wx-test"
            app_secret_env = "WECHAT_APP_SECRET"

            [jwt]
            secret_env = "JWT_SECRET"

            [database]
            url_env = "DATABASE_URL"

            [[upstreams]]
            prefix = "api"
            base_url = "http://127.0.0.1:9001"
            "#,
        )
        .expect_err("prefix without leading slash should fail");

        assert!(matches!(error, ConfigError::InvalidUpstreamPrefix(_)));
    }
}
