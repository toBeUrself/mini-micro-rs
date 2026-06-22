//! TOML 配置解析、Feature Flag、参数管理。

use serde::Deserialize;

/// klines-tools 顶层配置。
#[derive(Debug, Clone, Deserialize)]
pub struct KlinesToolsConfig {
    /// 服务绑定地址。
    #[serde(default = "default_bind")]
    pub bind: String,
    /// app-api 基础 URL。
    #[serde(default = "default_app_api_base_url")]
    pub app_api_base_url: String,
    /// HTTP 请求超时（秒）。
    #[serde(default = "default_http_timeout_secs")]
    pub http_timeout_secs: u64,
    /// 默认数据源。
    #[serde(default = "default_source")]
    pub default_source: String,

    /// Feature flags。
    #[serde(default)]
    pub features: FeatureFlags,

    /// 指标参数。
    #[serde(default)]
    pub indicator: IndicatorConfig,

    /// 状态机参数。
    #[serde(default)]
    pub state: StateConfig,

    /// 网格参数。
    #[serde(default)]
    pub grid: GridConfig,

    /// 风险参数。
    #[serde(default)]
    pub risk: RiskConfig,

    /// 数据质量阈值。
    #[serde(default)]
    pub data_quality: DataQualityConfig,

    /// 多周期配置。
    #[serde(default)]
    pub multi_timeframe: MultiTimeframeConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FeatureFlags {
    #[serde(default)]
    pub enable_multi_timeframe: bool,
    #[serde(default)]
    pub enable_donchian: bool,
    #[serde(default = "default_true")]
    pub enable_percent_b: bool,
    #[serde(default = "default_true")]
    pub enable_ema20_deviation: bool,
    #[serde(default)]
    pub enable_score_momentum: bool,
    #[serde(default = "default_true")]
    pub enable_score_conflict_adjustment: bool,
    #[serde(default)]
    pub enable_fake_breakout_filter: bool,
    #[serde(default)]
    pub enable_exchange_constraints: bool,
    #[serde(default)]
    pub enable_keltner: bool,
    #[serde(default)]
    pub enable_obv: bool,
    #[serde(default)]
    pub enable_vwap_deviation: bool,
    #[serde(default)]
    pub enable_ml_classifier: bool,
    #[serde(default)]
    pub enable_orderbook_features: bool,
}
impl Default for FeatureFlags {
    fn default() -> Self {
        Self {
            enable_multi_timeframe: false,
            enable_donchian: false,
            enable_percent_b: true,
            enable_ema20_deviation: true,
            enable_score_momentum: false,
            enable_score_conflict_adjustment: true,
            enable_fake_breakout_filter: false,
            enable_exchange_constraints: false,
            enable_keltner: false,
            enable_obv: false,
            enable_vwap_deviation: false,
            enable_ml_classifier: false,
            enable_orderbook_features: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct IndicatorConfig {
    #[serde(default = "default_boll_period")]
    pub boll_period: usize,
    #[serde(default = "default_boll_mult")]
    pub boll_mult: f64,
    #[serde(default = "default_macd_fast")]
    pub macd_fast: usize,
    #[serde(default = "default_macd_slow")]
    pub macd_slow: usize,
    #[serde(default = "default_macd_signal")]
    pub macd_signal: usize,
    #[serde(default = "default_atr_period")]
    pub atr_period: usize,
    #[serde(default = "default_adx_period")]
    pub adx_period: usize,
    #[serde(default = "default_rsi_period")]
    pub rsi_period: usize,
    #[serde(default = "default_vol_ma_period")]
    pub volume_ma_period: usize,
    #[serde(default = "default_donchian_period")]
    pub donchian_period: usize,
    #[serde(default = "default_score_smooth_period")]
    pub score_smooth_period: usize,
    #[serde(default = "default_percentile_window")]
    pub percentile_window: usize,
    #[serde(default = "default_percentile_min_samples")]
    pub percentile_min_samples: usize,
    #[serde(default = "default_pivot_left")]
    pub pivot_left: usize,
    #[serde(default = "default_pivot_right")]
    pub pivot_right: usize,
    #[serde(default = "default_structure_lookback")]
    pub structure_lookback: usize,
}
impl Default for IndicatorConfig {
    fn default() -> Self {
        Self {
            boll_period: 20,
            boll_mult: 2.0,
            macd_fast: 12,
            macd_slow: 26,
            macd_signal: 9,
            atr_period: 14,
            adx_period: 14,
            rsi_period: 14,
            volume_ma_period: 20,
            donchian_period: 20,
            score_smooth_period: 3,
            percentile_window: 1000,
            percentile_min_samples: 100,
            pivot_left: 2,
            pivot_right: 2,
            structure_lookback: 20,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct StateConfig {
    #[serde(default = "default_range_enter")]
    pub range_enter: f64,
    #[serde(default = "default_range_exit")]
    pub range_exit: f64,
    #[serde(default = "default_warning_enter")]
    pub warning_enter: f64,
    #[serde(default = "default_warning_exit")]
    pub warning_exit: f64,
    #[serde(default = "default_trend_candidate")]
    pub trend_candidate: f64,
    #[serde(default = "default_trend_confirm")]
    pub trend_confirm: f64,
    #[serde(default = "default_confirm_bars")]
    pub confirm_bars: usize,
    #[serde(default = "default_fake_breakout_window")]
    pub fake_breakout_window: usize,
    #[serde(default = "default_breakout_volume_confirm_threshold")]
    pub breakout_volume_confirm_threshold: f64,
    #[serde(default = "default_cooldown_bars_after_exit")]
    pub cooldown_bars_after_exit: usize,
    #[serde(default = "default_cooldown_bars_after_stop_loss")]
    pub cooldown_bars_after_stop_loss: usize,
}
impl Default for StateConfig {
    fn default() -> Self {
        Self {
            range_enter: 65.0,
            range_exit: 55.0,
            warning_enter: 55.0,
            warning_exit: 45.0,
            trend_candidate: 70.0,
            trend_confirm: 80.0,
            confirm_bars: 3,
            fake_breakout_window: 3,
            breakout_volume_confirm_threshold: 1.5,
            cooldown_bars_after_exit: 5,
            cooldown_bars_after_stop_loss: 20,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct GridConfig {
    #[serde(default = "default_grid_count")]
    pub grid_count: usize,
    #[serde(default = "default_boundary_mode")]
    pub boundary_mode: String,
    #[serde(default = "default_min_profit_buffer")]
    pub min_profit_buffer: f64,
    #[serde(default = "default_fee_rate")]
    pub fee_rate: f64,
    #[serde(default = "default_expected_slippage_rate")]
    pub expected_slippage_rate: f64,
    #[serde(default = "default_max_grid_width_pct")]
    pub max_grid_width_by_percent: f64,
    #[serde(default = "default_max_grid_width_atr")]
    pub max_grid_width_by_atr: f64,
    #[serde(default = "default_max_capital_usage")]
    pub max_capital_usage_at_lower_bound: f64,
}
impl Default for GridConfig {
    fn default() -> Self {
        Self {
            grid_count: 20,
            boundary_mode: "boll".into(),
            min_profit_buffer: 0.001,
            fee_rate: 0.001,
            expected_slippage_rate: 0.0005,
            max_grid_width_by_percent: 0.08,
            max_grid_width_by_atr: 6.0,
            max_capital_usage_at_lower_bound: 0.5,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RiskConfig {
    #[serde(default = "default_max_position_ratio")]
    pub max_position_ratio: f64,
    #[serde(default = "default_max_grid_capital")]
    pub max_grid_capital: f64,
    #[serde(default = "default_max_loss_per_symbol")]
    pub max_loss_per_symbol: f64,
    #[serde(default = "default_max_daily_loss")]
    pub max_daily_loss: f64,
    #[serde(default = "default_max_drawdown")]
    pub max_drawdown: f64,
    #[serde(default = "default_default_reduce_position_ratio")]
    pub default_reduce_position_ratio: f64,
}
impl Default for RiskConfig {
    fn default() -> Self {
        Self {
            max_position_ratio: 0.3,
            max_grid_capital: 1000.0,
            max_loss_per_symbol: 0.03,
            max_daily_loss: 0.05,
            max_drawdown: 0.1,
            default_reduce_position_ratio: 0.3,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DataQualityConfig {
    #[serde(default = "default_max_latest_delay_intervals")]
    pub max_latest_delay_intervals: i64,
    #[serde(default = "default_max_missing_kline_ratio")]
    pub max_missing_kline_ratio: f64,
    #[serde(default = "default_min_quality_score")]
    pub min_quality_score_for_grid: f64,
}
impl Default for DataQualityConfig {
    fn default() -> Self {
        Self {
            max_latest_delay_intervals: 2,
            max_missing_kline_ratio: 0.01,
            min_quality_score_for_grid: 0.9,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MultiTimeframeConfig {
    /// 多周期组合模式：short_watch / normal_grid / high_freq / intraday_risk
    #[serde(default = "default_tf_mode")]
    pub mode: String,
    /// higher 周期，如 "4h"
    pub higher_interval: Option<String>,
    /// middle 周期，如 "30m"
    pub middle_interval: Option<String>,
    /// lower 周期，如 "5m"
    pub lower_interval: Option<String>,
}
impl Default for MultiTimeframeConfig {
    fn default() -> Self {
        Self {
            mode: "normal_grid".into(),
            higher_interval: Some("4h".into()),
            middle_interval: Some("30m".into()),
            lower_interval: Some("5m".into()),
        }
    }
}

impl MultiTimeframeConfig {
    /// 返回 (higher, middle, lower) 周期三元组。
    pub fn intervals(&self) -> (Option<&str>, Option<&str>, Option<&str>) {
        (
            self.higher_interval.as_deref(),
            self.middle_interval.as_deref(),
            self.lower_interval.as_deref(),
        )
    }
}

// ── 默认值辅助函数 ────────────────────────────────────────────────────

fn default_bind() -> String { "127.0.0.1:8081".into() }
fn default_app_api_base_url() -> String { "http://127.0.0.1:8080".into() }
fn default_http_timeout_secs() -> u64 { 30 }
fn default_source() -> String { "binance".into() }

fn default_true() -> bool { true }
fn default_boll_period() -> usize { 20 }
fn default_boll_mult() -> f64 { 2.0 }
fn default_macd_fast() -> usize { 12 }
fn default_macd_slow() -> usize { 26 }
fn default_macd_signal() -> usize { 9 }
fn default_atr_period() -> usize { 14 }
fn default_adx_period() -> usize { 14 }
fn default_rsi_period() -> usize { 14 }
fn default_vol_ma_period() -> usize { 20 }
fn default_donchian_period() -> usize { 20 }
fn default_score_smooth_period() -> usize { 3 }
fn default_percentile_window() -> usize { 1000 }
fn default_percentile_min_samples() -> usize { 100 }
fn default_pivot_left() -> usize { 2 }
fn default_pivot_right() -> usize { 2 }
fn default_structure_lookback() -> usize { 20 }

fn default_range_enter() -> f64 { 65.0 }
fn default_range_exit() -> f64 { 55.0 }
fn default_warning_enter() -> f64 { 55.0 }
fn default_warning_exit() -> f64 { 45.0 }
fn default_trend_candidate() -> f64 { 70.0 }
fn default_trend_confirm() -> f64 { 80.0 }
fn default_confirm_bars() -> usize { 3 }
fn default_fake_breakout_window() -> usize { 3 }
fn default_breakout_volume_confirm_threshold() -> f64 { 1.5 }
fn default_cooldown_bars_after_exit() -> usize { 5 }
fn default_cooldown_bars_after_stop_loss() -> usize { 20 }

fn default_grid_count() -> usize { 20 }
fn default_boundary_mode() -> String { "boll".into() }
fn default_min_profit_buffer() -> f64 { 0.001 }
fn default_fee_rate() -> f64 { 0.001 }
fn default_expected_slippage_rate() -> f64 { 0.0005 }
fn default_max_grid_width_pct() -> f64 { 0.08 }
fn default_max_grid_width_atr() -> f64 { 6.0 }
fn default_max_capital_usage() -> f64 { 0.5 }

fn default_max_position_ratio() -> f64 { 0.3 }
fn default_max_grid_capital() -> f64 { 1000.0 }
fn default_max_loss_per_symbol() -> f64 { 0.03 }
fn default_max_daily_loss() -> f64 { 0.05 }
fn default_max_drawdown() -> f64 { 0.1 }
fn default_default_reduce_position_ratio() -> f64 { 0.3 }

fn default_max_latest_delay_intervals() -> i64 { 2 }
fn default_max_missing_kline_ratio() -> f64 { 0.01 }
fn default_min_quality_score() -> f64 { 0.9 }

fn default_tf_mode() -> String { "normal_grid".into() }

// ── 配置加载 ──────────────────────────────────────────────────────────

impl KlinesToolsConfig {
    /// 从 TOML 字符串解析配置。
    pub fn parse(toml_str: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(toml_str)
    }

    /// 获取启用的 feature 列表。
    pub fn enabled_features(&self) -> Vec<String> {
        let mut features = Vec::new();
        if self.features.enable_multi_timeframe { features.push("multi_timeframe".into()); }
        if self.features.enable_donchian { features.push("donchian".into()); }
        if self.features.enable_percent_b { features.push("percent_b".into()); }
        if self.features.enable_ema20_deviation { features.push("ema20_deviation".into()); }
        if self.features.enable_score_momentum { features.push("score_momentum".into()); }
        if self.features.enable_score_conflict_adjustment { features.push("score_conflict_adjustment".into()); }
        if self.features.enable_fake_breakout_filter { features.push("fake_breakout_filter".into()); }
        if self.features.enable_exchange_constraints { features.push("exchange_constraints".into()); }
        if self.features.enable_keltner { features.push("keltner".into()); }
        if self.features.enable_obv { features.push("obv".into()); }
        if self.features.enable_vwap_deviation { features.push("vwap_deviation".into()); }
        if self.features.enable_ml_classifier { features.push("ml_classifier".into()); }
        if self.features.enable_orderbook_features { features.push("orderbook_features".into()); }
        features
    }

    /// 计算配置哈希。
    pub fn config_hash(&self) -> String {
        use sha2::{Digest, Sha256};
        let json = serde_json::to_string(&serde_json::json!({
            "features": self.enabled_features(),
            "indicator": {
                "boll_period": self.indicator.boll_period,
                "boll_mult": self.indicator.boll_mult,
                "macd_fast": self.indicator.macd_fast,
                "macd_slow": self.indicator.macd_slow,
                "macd_signal": self.indicator.macd_signal,
                "atr_period": self.indicator.atr_period,
                "adx_period": self.indicator.adx_period,
                "rsi_period": self.indicator.rsi_period,
                "volume_ma_period": self.indicator.volume_ma_period,
                "donchian_period": self.indicator.donchian_period,
                "score_smooth_period": self.indicator.score_smooth_period,
            },
            "state": {
                "range_enter": self.state.range_enter,
                "range_exit": self.state.range_exit,
                "warning_enter": self.state.warning_enter,
                "warning_exit": self.state.warning_exit,
                "trend_candidate": self.state.trend_candidate,
                "trend_confirm": self.state.trend_confirm,
                "confirm_bars": self.state.confirm_bars,
                "fake_breakout_window": self.state.fake_breakout_window,
                "cooldown_bars_after_exit": self.state.cooldown_bars_after_exit,
            },
        }))
        .unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(json.as_bytes());
        format!("sha256:{:x}", hasher.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let features = FeatureFlags::default();
        assert!(features.enable_score_conflict_adjustment);
        assert!(!features.enable_multi_timeframe);
    }

    #[test]
    fn test_minimal_toml() {
        let toml_str = r#"
bind = "0.0.0.0:8081"
[features]
enable_donchian = true
[indicator]
boll_period = 20
"#;
        let config = KlinesToolsConfig::parse(toml_str).expect("should parse");
        assert_eq!(config.bind, "0.0.0.0:8081");
        assert!(config.features.enable_donchian);
        assert!(!config.features.enable_multi_timeframe);
        assert_eq!(config.indicator.boll_period, 20);
    }
}
