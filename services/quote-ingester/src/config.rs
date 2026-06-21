use std::{env, fs, path::Path, time::Duration};

use serde::Deserialize;
use thiserror::Error;

/// quote-ingester 的原始 TOML 配置。
///
/// 这个结构体直接对应 `quote-ingester.toml` 的文件结构。
/// `Deserialize` 让 serde/toml 可以把文本配置自动解析成 Rust 结构体。
#[derive(Debug, Clone, Deserialize)]
pub struct QuoteIngesterConfig {
    /// 数据库配置。
    pub database: DatabaseConfig,
    /// 行情 API 配置。没有写时使用默认值。
    #[serde(default)]
    pub quote_api: QuoteApiConfig,
    /// 要采集的市场列表。一个 market 通常表示一个交易对 + 一个源周期。
    #[serde(default)]
    pub markets: Vec<MarketConfig>,
}

/// 数据库相关配置。
#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    /// 保存数据库连接串的环境变量名，例如 `DATABASE_URL`。
    ///
    /// TOML 里只保存环境变量名，不直接保存密码，避免把敏感信息写进代码仓库。
    pub url_env: String,
}

/// 行情接口配置。
#[derive(Debug, Clone, Deserialize)]
pub struct QuoteApiConfig {
    /// 数据源标识，会写入 `klines.source`。
    #[serde(default = "default_source")]
    pub source: String,
    /// Binance API 域名。
    #[serde(default = "default_base_url")]
    pub base_url: String,
    /// HTTP 请求超时时间，单位秒。
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
}

/// 单个采集任务的配置。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct MarketConfig {
    /// Binance 交易对格式，比如 `BTCUSDT`。
    pub symbol: String,
    /// 从远端直接拉取的周期。当前默认拉 `1m`。
    #[serde(default = "default_source_interval")]
    pub source_interval: String,
    /// 单次请求最多返回多少根 K 线。Binance 当前最大常用值是 1000。
    #[serde(default = "default_limit")]
    pub limit: u32,
    /// 实时轮询间隔，单位秒。
    #[serde(default = "default_poll_seconds")]
    pub poll_seconds: u64,
    /// 启动时尝试回填多少天历史数据。
    #[serde(default = "default_backfill_days")]
    pub backfill_days: u32,
    /// 从 `source_interval` 本地聚合出来的目标周期。
    #[serde(default = "default_derived_intervals")]
    pub derived_intervals: Vec<String>,
}

/// 已解析、已校验、可直接用于运行服务的配置。
///
/// 和 `QuoteIngesterConfig` 的区别：
/// - 这里已经从环境变量里读出了 `database_url`。
/// - `timeout_seconds` 已经转成了 `Duration`，方便 reqwest 使用。
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    /// Postgres 连接字符串。
    pub database_url: String,
    /// 数据源标识，比如 `binance`。
    pub source: String,
    /// API 基础 URL。
    pub api_base_url: String,
    /// HTTP 请求超时时间。
    pub api_timeout: Duration,
    /// 采集任务列表。
    pub markets: Vec<MarketConfig>,
}

/// 配置层可能出现的错误。
///
/// `thiserror::Error` 可以帮我们把 enum 自动实现成标准错误类型。
#[derive(Debug, Error)]
pub enum ConfigError {
    /// 配置文件读取失败。
    #[error("failed to read config file {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    /// TOML 语法或字段类型解析失败。
    #[error("failed to parse config TOML: {0}")]
    Toml(#[from] toml::de::Error),
    /// 必需的环境变量不存在。
    #[error("required environment variable {name} is not set")]
    MissingEnv { name: String },
    /// 环境变量存在但值为空字符串。
    #[error("required environment variable {name} is empty")]
    EmptyEnv { name: String },
    /// 数据源名称为空。
    #[error("quote_api.source must not be empty")]
    EmptySource,
    /// API 地址不是 http/https URL。
    #[error("quote_api.base_url must start with http:// or https://: {0}")]
    InvalidBaseUrl(String),
    /// 超时时间必须大于 0。
    #[error("quote_api.timeout_seconds must be greater than 0")]
    InvalidTimeout,
    /// 至少需要配置一个 market。
    #[error("at least one market must be configured")]
    NoMarkets,
    /// 交易对不能为空。
    #[error("market symbol must not be empty")]
    EmptySymbol,
    /// 周期格式无效，比如 `bad` 或 `0m`。
    #[error("invalid interval: {0}")]
    InvalidInterval(String),
    /// limit 必须大于 0。
    #[error("market limit must be greater than 0")]
    InvalidLimit,
    /// 轮询间隔必须大于 0。
    #[error("market poll_seconds must be greater than 0")]
    InvalidPollSeconds,
    /// 回填天数必须大于 0。
    #[error("market backfill_days must be greater than 0")]
    InvalidBackfillDays,
    /// 目标聚合周期必须能被源周期整除。
    #[error("derived interval {derived} must be a multiple of source interval {source_interval}")]
    DerivedIntervalNotMultiple {
        source_interval: String,
        derived: String,
    },
    /// 目标聚合周期必须比源周期更长。
    #[error("derived interval {derived} must be longer than source interval {source_interval}")]
    DerivedIntervalNotLonger {
        source_interval: String,
        derived: String,
    },
}

impl QuoteIngesterConfig {
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
    /// 测试里常用这个方法，因为不用真的创建配置文件。
    pub fn from_toml_str(content: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(content)?;
        config.validate()?;
        Ok(config)
    }

    /// 把原始配置转换成运行时配置。
    ///
    /// 这里会读取环境变量，因此可能返回 MissingEnv 或 EmptyEnv。
    pub fn resolve(self) -> Result<ResolvedConfig, ConfigError> {
        self.validate()?;
        Ok(ResolvedConfig {
            database_url: required_env(&self.database.url_env)?,
            source: self.quote_api.source,
            api_base_url: self.quote_api.base_url,
            api_timeout: Duration::from_secs(self.quote_api.timeout_seconds),
            markets: self.markets,
        })
    }

    /// 校验配置里的业务规则。
    ///
    /// Rust 类型只能保证字段类型正确，比如 `limit` 是数字；
    /// 但不能保证 `limit > 0` 或 `5m` 能由 `1m` 聚合，这些规则要在这里检查。
    fn validate(&self) -> Result<(), ConfigError> {
        if self.quote_api.source.trim().is_empty() {
            return Err(ConfigError::EmptySource);
        }
        if !self.quote_api.base_url.starts_with("http://")
            && !self.quote_api.base_url.starts_with("https://")
        {
            return Err(ConfigError::InvalidBaseUrl(self.quote_api.base_url.clone()));
        }
        if self.quote_api.timeout_seconds == 0 {
            return Err(ConfigError::InvalidTimeout);
        }
        if self.markets.is_empty() {
            return Err(ConfigError::NoMarkets);
        }

        for market in &self.markets {
            if market.symbol.trim().is_empty() {
                return Err(ConfigError::EmptySymbol);
            }
            if market.limit == 0 {
                return Err(ConfigError::InvalidLimit);
            }
            if market.poll_seconds == 0 {
                return Err(ConfigError::InvalidPollSeconds);
            }
            if market.backfill_days == 0 {
                return Err(ConfigError::InvalidBackfillDays);
            }

            let source_minutes = interval_minutes(&market.source_interval)
                .ok_or_else(|| ConfigError::InvalidInterval(market.source_interval.clone()))?;

            for derived in &market.derived_intervals {
                let derived_minutes = interval_minutes(derived)
                    .ok_or_else(|| ConfigError::InvalidInterval(derived.clone()))?;
                if derived_minutes <= source_minutes {
                    return Err(ConfigError::DerivedIntervalNotLonger {
                        source_interval: market.source_interval.clone(),
                        derived: derived.clone(),
                    });
                }
                if derived_minutes % source_minutes != 0 {
                    return Err(ConfigError::DerivedIntervalNotMultiple {
                        source_interval: market.source_interval.clone(),
                        derived: derived.clone(),
                    });
                }
            }
        }

        Ok(())
    }
}

/// 把周期字符串转成分钟数。
///
/// 例如：
/// - `1m` -> 1
/// - `2h` -> 120
/// - `1d` -> 1440
pub fn interval_minutes(interval: &str) -> Option<i64> {
    let (number, unit) = interval.split_at(interval.len().checked_sub(1)?);
    let value = number.parse::<i64>().ok()?;
    if value <= 0 {
        return None;
    }

    match unit {
        "m" => Some(value),
        "h" => Some(value * 60),
        "d" => Some(value * 60 * 24),
        _ => None,
    }
}

/// 读取必需的环境变量。
///
/// 返回 `Result<String, ConfigError>` 的原因是：环境变量可能不存在，也可能是空字符串。
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

/// 默认数据源名称。
fn default_source() -> String {
    "binance".to_string()
}

/// 默认 Binance API 域名。
fn default_base_url() -> String {
    "https://www.binance.com".to_string()
}

/// 默认 HTTP 超时时间，单位秒。
fn default_timeout_seconds() -> u64 {
    10
}

/// 默认远端拉取周期。
fn default_source_interval() -> String {
    "1m".to_string()
}

/// 默认单次请求数量。
fn default_limit() -> u32 {
    1000
}

/// 默认轮询间隔，单位秒。
fn default_poll_seconds() -> u64 {
    20
}

/// 默认启动回填天数。
fn default_backfill_days() -> u32 {
    30
}

/// 默认由 1m 聚合出的周期。
fn default_derived_intervals() -> Vec<String> {
    vec!["5m".to_string(), "30m".to_string()]
}

impl Default for QuoteApiConfig {
    /// 给 `QuoteApiConfig` 提供 serde 可以使用的默认值。
    fn default() -> Self {
        Self {
            source: default_source(),
            base_url: default_base_url(),
            timeout_seconds: default_timeout_seconds(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_config() {
        let config = QuoteIngesterConfig::from_toml_str(
            r#"
            [database]
            url_env = "DATABASE_URL"

            [quote_api]
            source = "binance"
            base_url = "https://www.binance.com"
            timeout_seconds = 10

            [[markets]]
            symbol = "BTCUSDT"
            source_interval = "1m"
            limit = 1000
            poll_seconds = 20
            backfill_days = 30
            derived_intervals = ["5m", "30m"]
            "#,
        )
        .expect("config should parse");

        assert_eq!(config.database.url_env, "DATABASE_URL");
        assert_eq!(config.quote_api.source, "binance");
        assert_eq!(
            config.markets,
            vec![MarketConfig {
                symbol: "BTCUSDT".to_string(),
                source_interval: "1m".to_string(),
                limit: 1000,
                poll_seconds: 20,
                backfill_days: 30,
                derived_intervals: vec!["5m".to_string(), "30m".to_string()],
            }]
        );
    }

    #[test]
    fn rejects_invalid_derived_interval() {
        let error = QuoteIngesterConfig::from_toml_str(
            r#"
            [database]
            url_env = "DATABASE_URL"

            [[markets]]
            symbol = "BTCUSDT"
            source_interval = "5m"
            derived_intervals = ["1m"]
            "#,
        )
        .expect_err("shorter derived interval should fail");

        assert!(matches!(
            error,
            ConfigError::DerivedIntervalNotLonger { .. }
        ));
    }

    #[test]
    fn parses_interval_minutes() {
        assert_eq!(interval_minutes("1m"), Some(1));
        assert_eq!(interval_minutes("2h"), Some(120));
        assert_eq!(interval_minutes("1d"), Some(1440));
        assert_eq!(interval_minutes("0m"), None);
        assert_eq!(interval_minutes("bad"), None);
    }
}
