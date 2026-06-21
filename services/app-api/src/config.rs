use std::{env, fs, net::SocketAddr, path::Path};

use serde::Deserialize;
use thiserror::Error;

/// app-api 的原始配置。
///
/// 这个结构体直接对应 TOML 文件内容。
#[derive(Debug, Clone, Deserialize)]
pub struct AppApiConfig {
    /// HTTP 服务配置。
    pub server: ServerConfig,
    /// 数据库配置。
    pub database: DatabaseConfig,
}

/// HTTP 服务配置。
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// 监听地址，比如 `127.0.0.1:9000`。
    pub bind: SocketAddr,
}

/// 数据库配置。
#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    /// 存放数据库连接串的环境变量名字。
    ///
    /// 注意这里不是直接写数据库密码，而是写环境变量名，比如 `DATABASE_URL`。
    pub url_env: String,
}

/// 解析环境变量后的最终配置。
///
/// 原始 TOML 里只有 `url_env = "DATABASE_URL"`，
/// `resolve()` 后这里才会拿到真正的数据库连接串。
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    /// HTTP 监听地址。
    pub bind: SocketAddr,
    /// Postgres 连接串。
    pub database_url: String,
}

/// 配置读取和解析过程中可能出现的错误。
#[derive(Debug, Error)]
pub enum ConfigError {
    /// 配置文件读取失败。
    #[error("failed to read config file {path}: {source}")]
    Io {
        /// 配置文件路径。
        path: String,
        /// 底层 IO 错误。
        #[source]
        source: std::io::Error,
    },
    /// TOML 格式不正确。
    #[error("failed to parse config TOML: {0}")]
    Toml(#[from] toml::de::Error),
    /// 必要环境变量没有设置。
    #[error("required environment variable {name} is not set")]
    MissingEnv {
        /// 环境变量名。
        name: String,
    },
    /// 必要环境变量设置了，但是值为空字符串。
    #[error("required environment variable {name} is empty")]
    EmptyEnv {
        /// 环境变量名。
        name: String,
    },
}

impl AppApiConfig {
    /// 从 TOML 文件读取配置。
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path_ref = path.as_ref();
        let content = fs::read_to_string(path_ref).map_err(|source| ConfigError::Io {
            path: path_ref.display().to_string(),
            source,
        })?;
        Self::from_toml_str(&content)
    }

    /// 从 TOML 字符串解析配置。
    ///
    /// 这个方法主要方便单元测试，不需要真的创建文件。
    pub fn from_toml_str(content: &str) -> Result<Self, ConfigError> {
        Ok(toml::from_str(content)?)
    }

    /// 把“配置文件里的环境变量名”解析成“真正可用的配置值”。
    pub fn resolve(self) -> Result<ResolvedConfig, ConfigError> {
        Ok(ResolvedConfig {
            bind: self.server.bind,
            database_url: required_env(&self.database.url_env)?,
        })
    }
}

/// 读取一个必填环境变量。
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_config() {
        let config = AppApiConfig::from_toml_str(
            r#"
            [server]
            bind = "127.0.0.1:9000"

            [database]
            url_env = "DATABASE_URL"
            "#,
        )
        .expect("config should parse");

        assert_eq!(config.server.bind.to_string(), "127.0.0.1:9000");
        assert_eq!(config.database.url_env, "DATABASE_URL");
    }
}
