use std::{env, fs, path::Path, time::Duration};

use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Clone, Deserialize)]
pub struct QuoteIngesterConfig {
    pub database: DatabaseConfig,
    #[serde(default)]
    pub quote_api: QuoteApiConfig,
    #[serde(default)]
    pub markets: Vec<MarketConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub url_env: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QuoteApiConfig {
    #[serde(default = "default_source")]
    pub source: String,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct MarketConfig {
    pub symbol: String,
    #[serde(default = "default_source_interval")]
    pub source_interval: String,
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default = "default_poll_seconds")]
    pub poll_seconds: u64,
    #[serde(default = "default_backfill_days")]
    pub backfill_days: u32,
    #[serde(default = "default_derived_intervals")]
    pub derived_intervals: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub database_url: String,
    pub source: String,
    pub api_base_url: String,
    pub api_timeout: Duration,
    pub markets: Vec<MarketConfig>,
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
    #[error("quote_api.source must not be empty")]
    EmptySource,
    #[error("quote_api.base_url must start with http:// or https://: {0}")]
    InvalidBaseUrl(String),
    #[error("quote_api.timeout_seconds must be greater than 0")]
    InvalidTimeout,
    #[error("at least one market must be configured")]
    NoMarkets,
    #[error("market symbol must not be empty")]
    EmptySymbol,
    #[error("invalid interval: {0}")]
    InvalidInterval(String),
    #[error("market limit must be greater than 0")]
    InvalidLimit,
    #[error("market poll_seconds must be greater than 0")]
    InvalidPollSeconds,
    #[error("market backfill_days must be greater than 0")]
    InvalidBackfillDays,
    #[error("derived interval {derived} must be a multiple of source interval {source_interval}")]
    DerivedIntervalNotMultiple {
        source_interval: String,
        derived: String,
    },
    #[error("derived interval {derived} must be longer than source interval {source_interval}")]
    DerivedIntervalNotLonger {
        source_interval: String,
        derived: String,
    },
}

impl QuoteIngesterConfig {
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
            database_url: required_env(&self.database.url_env)?,
            source: self.quote_api.source,
            api_base_url: self.quote_api.base_url,
            api_timeout: Duration::from_secs(self.quote_api.timeout_seconds),
            markets: self.markets,
        })
    }

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

fn default_source() -> String {
    "binance".to_string()
}

fn default_base_url() -> String {
    "https://www.binance.com".to_string()
}

fn default_timeout_seconds() -> u64 {
    10
}

fn default_source_interval() -> String {
    "1m".to_string()
}

fn default_limit() -> u32 {
    1000
}

fn default_poll_seconds() -> u64 {
    20
}

fn default_backfill_days() -> u32 {
    30
}

fn default_derived_intervals() -> Vec<String> {
    vec!["5m".to_string(), "30m".to_string()]
}

impl Default for QuoteApiConfig {
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
