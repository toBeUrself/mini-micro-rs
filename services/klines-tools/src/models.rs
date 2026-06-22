//! klines-tools 核心数据模型。
//!
//! 包含 K线、数据质量、指标、评分、状态机、风险决策、网格计划、信号等所有数据模型。

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

// ── K 线 ──────────────────────────────────────────────────────────────

/// 分析模块使用的 K 线模型。
///
/// 与 quote-ingester 的 Kline 不同，这里使用 `f64` 用于指标计算。
/// 执行层（GridLevel）仍使用 Decimal。
#[derive(Debug, Clone)]
pub struct Kline {
    /// 开盘时间（毫秒时间戳）。
    pub open_time: i64,
    /// K 线周期，如 "1m"、"5m"。
    pub interval: String,
    /// 开盘价。
    pub open: f64,
    /// 最高价。
    pub high: f64,
    /// 最低价。
    pub low: f64,
    /// 收盘价。
    pub close: f64,
    /// 成交量（基础币）。
    pub volume: f64,
    /// 成交额（计价币），可选。
    pub quote_volume: Option<f64>,
    /// 收盘时间（毫秒时间戳），可选。
    pub close_time: Option<i64>,
    /// 成交笔数，可选。
    pub trade_count: Option<u64>,
    /// 是否已闭合。
    pub is_closed: bool,
}

impl Kline {
    /// K 线周期的毫秒长度。
    pub fn interval_ms(&self) -> i64 {
        parse_interval_ms(&self.interval)
    }
}

/// 把 "1m"、"5m"、"1h" 等周期字符串转成毫秒数。
pub fn parse_interval_ms(interval: &str) -> i64 {
    let interval = interval.trim().to_ascii_lowercase();
    if interval.is_empty() {
        return 0;
    }
    let (num_part, unit_part) = interval.split_at(interval.len() - 1);
    let num: i64 = num_part.parse().unwrap_or(1);
    match unit_part {
        "s" => num * 1000,
        "m" => num * 60 * 1000,
        "h" => num * 3600 * 1000,
        "d" => num * 86400 * 1000,
        "w" => num * 604800 * 1000,
        _ => 0,
    }
}

// ── 数据质量 ──────────────────────────────────────────────────────────

/// K 线缺口范围。
#[derive(Debug, Clone, Serialize)]
pub struct GapRange {
    pub expected_open_time: i64,
    pub next_seen_open_time: i64,
    pub missing_count: usize,
}

/// 数据质量评估结果。
#[derive(Debug, Clone, Serialize)]
pub struct DataQuality {
    pub input_kline_count: usize,
    pub usable_closed_kline_count: usize,
    pub first_open_time: Option<i64>,
    pub last_open_time: Option<i64>,
    pub expected_interval_ms: i64,
    pub missing_kline_count: usize,
    pub missing_kline_ratio: f64,
    pub max_gap_bars: usize,
    pub gap_ranges: Vec<GapRange>,
    pub duplicate_kline_count: usize,
    pub out_of_order_count: usize,
    pub invalid_ohlcv_count: usize,
    pub has_gap: bool,
    pub has_unclosed_kline: bool,
    pub latest_kline_delay_ms: i64,
    pub warmup_satisfied: bool,
    /// 0.0 ~ 1.0
    pub quality_score: f64,
    pub issues: Vec<String>,
}

// ── 指标可用性 ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct IndicatorAvailability {
    pub ready: bool,
    pub min_required_bars: usize,
    pub warmup_bars: usize,
    pub unavailable_fields: Vec<String>,
}

// ── 评分系统 ──────────────────────────────────────────────────────────

/// 三维评分。
#[derive(Debug, Clone, Serialize, Default)]
pub struct Scores {
    pub range_score: f64,
    pub up_score: f64,
    pub down_score: f64,
}

/// 单个子维度的评分详情。
#[derive(Debug, Clone, Serialize)]
pub struct ScoreDetail {
    pub name: String,
    pub raw_value: Option<f64>,
    pub sub_score: Option<f64>,
    pub weight: f64,
    pub weighted_score: Option<f64>,
    pub available: bool,
    pub reason: String,
}

/// 三维评分的全部明细。
#[derive(Debug, Clone, Serialize)]
pub struct ScoreBreakdown {
    pub range: Vec<ScoreDetail>,
    pub up: Vec<ScoreDetail>,
    pub down: Vec<ScoreDetail>,
}

/// 评分动能。
#[derive(Debug, Clone, Serialize, Default)]
pub struct ScoreMomentum {
    pub range_momentum: f64,
    pub up_momentum: f64,
    pub down_momentum: f64,
}

// ── 状态机 ────────────────────────────────────────────────────────────

/// 市场状态：只表达市场行情状态，不表达风控覆盖。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketState {
    /// 方向不清晰或数据质量不足。
    Wait,
    /// 震荡条件成立，允许普通震荡网格。
    RangeGrid,
    /// 震荡可能向上失效。
    UpBreakWarning,
    /// 上涨趋势确认，关闭普通震荡网格。
    UptrendFollow,
    /// 震荡可能向下失效。
    DownBreakWarning,
    /// 下跌趋势风险确认，关闭普通网格。
    DowntrendRisk,
}

impl MarketState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Wait => "wait",
            Self::RangeGrid => "range_grid",
            Self::UpBreakWarning => "up_break_warning",
            Self::UptrendFollow => "uptrend_follow",
            Self::DownBreakWarning => "down_break_warning",
            Self::DowntrendRisk => "downtrend_risk",
        }
    }
}

/// 状态阶段：观察 → 候选 → 确认 → 冷却。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatePhase {
    Observing,
    Candidate,
    Confirmed,
    CoolingDown,
}

impl StatePhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Observing => "observing",
            Self::Candidate => "candidate",
            Self::Confirmed => "confirmed",
            Self::CoolingDown => "cooling_down",
        }
    }
}

/// 状态上下文：记录前序状态和计数信息。
#[derive(Debug, Clone)]
pub struct StateContext {
    pub previous_state: MarketState,
    pub previous_state_phase: StatePhase,
    pub previous_state_since: i64,
    pub candidate_state: Option<MarketState>,
    pub candidate_bars: usize,
    pub required_confirm_bars: usize,
    pub cooldown_remaining_bars: usize,
    pub last_transition_time: Option<i64>,
    pub last_grid_exit_time: Option<i64>,
    pub last_stop_loss_time: Option<i64>,
    /// 最后一次推进状态机时使用的已闭合 K 线 open_time。
    /// 用于防止同一根 K 线被重复推进 candidate_bars / cooldown。
    pub last_processed_open_time: Option<i64>,
}

impl Default for StateContext {
    fn default() -> Self {
        Self {
            previous_state: MarketState::Wait,
            previous_state_phase: StatePhase::Observing,
            previous_state_since: 0,
            candidate_state: None,
            candidate_bars: 0,
            required_confirm_bars: 3,
            cooldown_remaining_bars: 0,
            last_transition_time: None,
            last_grid_exit_time: None,
            last_stop_loss_time: None,
            last_processed_open_time: None,
        }
    }
}

/// 状态迁移记录。
#[derive(Debug, Clone, Serialize)]
pub struct StateTransition {
    pub previous_state: MarketState,
    pub candidate_state: Option<MarketState>,
    pub final_state: MarketState,
    pub final_state_phase: StatePhase,
    pub transition_type: String,
    pub candidate_bars: usize,
    pub cooldown_remaining_bars: usize,
    pub reasons: Vec<String>,
}

// ── 风险决策 ──────────────────────────────────────────────────────────

/// 风险覆盖层。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskOverride {
    None,
    GlobalHardStop,
    DataQualityBlock,
    IndicatorUnavailableBlock,
    ManualBlock,
    ExchangeConstraintBlock,
}

impl RiskOverride {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::GlobalHardStop => "global_hard_stop",
            Self::DataQualityBlock => "data_quality_block",
            Self::IndicatorUnavailableBlock => "indicator_unavailable_block",
            Self::ManualBlock => "manual_block",
            Self::ExchangeConstraintBlock => "exchange_constraint_block",
        }
    }
}

/// 风险等级。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Advisory,
    SoftBlock,
    HardBlock,
    EmergencyStop,
}

impl RiskLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Advisory => "advisory",
            Self::SoftBlock => "soft_block",
            Self::HardBlock => "hard_block",
            Self::EmergencyStop => "emergency_stop",
        }
    }
}

/// 允许的网格模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AllowedGridMode {
    RangeGrid,
    UptrendFollow,
}

/// 订单权限。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderPermission {
    None,
    ReadOnly,
    NewOrdersAllowed,
    ReplaceOnly,
    ReduceOnly,
}

impl OrderPermission {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ReadOnly => "read_only",
            Self::NewOrdersAllowed => "new_orders_allowed",
            Self::ReplaceOnly => "replace_only",
            Self::ReduceOnly => "reduce_only",
        }
    }
}

/// 仓位动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PositionAction {
    Hold,
    ReduceByRatio,
    StopLoss,
    CloseGridOnly,
    ManualReview,
}

impl PositionAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Hold => "hold",
            Self::ReduceByRatio => "reduce_by_ratio",
            Self::StopLoss => "stop_loss",
            Self::CloseGridOnly => "close_grid_only",
            Self::ManualReview => "manual_review",
        }
    }
}

/// 市场类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketType {
    Spot,
    UsdMarginedFutures,
    CoinMarginedFutures,
}

/// 可执行的风险决策。
#[derive(Debug, Clone, Serialize)]
pub struct RiskDecision {
    pub risk_level: RiskLevel,
    pub risk_override: RiskOverride,
    pub allowed_grid_modes: Vec<AllowedGridMode>,
    pub order_permission: OrderPermission,
    pub position_action: PositionAction,
    pub reduce_position_ratio: Option<Decimal>,
    pub reduce_reference: Option<String>,
    pub require_manual_confirm: bool,
    pub action_ttl_ms: i64,
    pub expire_at: i64,
    pub reasons: Vec<String>,
}

/// 投资组合风险输入（外部账户风控层提供）。
#[derive(Debug, Clone)]
pub struct PortfolioRiskInput {
    pub account_equity: Decimal,
    pub symbol_position_qty: Decimal,
    pub symbol_position_notional: Decimal,
    pub avg_entry_price: Option<Decimal>,
    pub unrealized_pnl: Option<Decimal>,
    pub realized_pnl_today: Option<Decimal>,
    pub max_equity_drawdown: Option<f64>,
    pub grid_capital_used: Decimal,
    pub open_order_count: usize,
}

// ── 多周期 ────────────────────────────────────────────────────────────

/// 单个周期的分析快照引用。
#[derive(Debug, Clone, Serialize)]
pub struct TimeframeSnapshotRef {
    pub source: String,
    pub symbol: String,
    pub interval: String,
    pub open_time: i64,
    pub close_time: i64,
    pub is_closed: bool,
    pub state: MarketState,
    pub state_phase: StatePhase,
    pub candidate_bars: usize,
    pub required_confirm_bars: usize,
    pub cooldown_remaining_bars: usize,
    pub raw_scores: Scores,
    pub smoothed_scores: Scores,
    pub confidence: f64,
}

// ── 网格计划 ──────────────────────────────────────────────────────────

/// 网格模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GridMode {
    Wait,
    RangeGrid,
    UptrendFollow,
    RiskControl,
    StopOrReduce,
}

impl GridMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Wait => "wait",
            Self::RangeGrid => "range_grid",
            Self::UptrendFollow => "uptrend_follow",
            Self::RiskControl => "risk_control",
            Self::StopOrReduce => "stop_or_reduce",
        }
    }
}

/// 展示阶段的网格计划（MVP）。
#[derive(Debug, Clone, Serialize)]
pub struct DisplayGridPlan {
    pub enabled: bool,
    pub mode: GridMode,
    pub boundary_mode: String,
    pub lower: Option<f64>,
    pub upper: Option<f64>,
    pub center: Option<f64>,
    pub grid_count: usize,
    pub grid_step: Option<f64>,
    pub risk_level: RiskLevel,
    pub confidence: f64,
}

/// 订单方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderSide {
    Buy,
    Sell,
}

/// 单个网格层（准实盘/执行契约必须使用此结构）。
#[derive(Debug, Clone, Serialize)]
pub struct GridLevel {
    pub side: OrderSide,
    /// 原始价格（Decimal）。
    pub raw_price: Decimal,
    /// 按 tick_size 取整后的 Decimal 价格。
    pub price: Decimal,
    /// 原始数量（Decimal）。
    pub raw_qty: Decimal,
    /// 按 step_size 取整后的 Decimal 数量。
    pub qty: Decimal,
    pub notional: Decimal,
    pub executable: bool,
    pub disabled_reason: Option<String>,
}

/// 可执行的网格计划。
#[derive(Debug, Clone, Serialize)]
pub struct ExecutableGridPlan {
    pub enabled: bool,
    pub mode: GridMode,
    pub levels: Vec<GridLevel>,
    pub total_required_capital: Decimal,
    pub executable_level_count: usize,
}

/// 交易所约束。
#[derive(Debug, Clone)]
pub struct ExchangeConstraints {
    pub tick_size: Decimal,
    pub step_size: Decimal,
    pub min_qty: Decimal,
    pub min_notional: Decimal,
    pub price_precision: u32,
    pub quantity_precision: u32,
    pub max_open_orders: Option<usize>,
    pub maker_fee_rate: Decimal,
    pub taker_fee_rate: Decimal,
}

// ── 置信度 ────────────────────────────────────────────────────────────

/// 置信度分解。
#[derive(Debug, Clone, Serialize)]
pub struct ConfidenceBreakdown {
    pub state_evidence: f64,
    pub data_quality: f64,
    pub indicator_availability: f64,
    pub timeframe_alignment: f64,
    pub state_stability: f64,
    pub final_confidence: f64,
}

// ── 信号 ──────────────────────────────────────────────────────────────

/// 信号类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalType {
    GridBuyWatch,
    GridSellWatch,
    UpBreakWarning,
    DownBreakWarning,
    PauseGrid,
    ResumeGrid,
    MoveGridUp,
    MoveGridDown,
    RiskReduce,
}

impl SignalType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GridBuyWatch => "grid_buy_watch",
            Self::GridSellWatch => "grid_sell_watch",
            Self::UpBreakWarning => "up_break_warning",
            Self::DownBreakWarning => "down_break_warning",
            Self::PauseGrid => "pause_grid",
            Self::ResumeGrid => "resume_grid",
            Self::MoveGridUp => "move_grid_up",
            Self::MoveGridDown => "move_grid_down",
            Self::RiskReduce => "risk_reduce",
        }
    }
}

/// 单个信号。
#[derive(Debug, Clone, Serialize)]
pub struct Signal {
    pub time: i64,
    pub price: f64,
    pub signal_type: SignalType,
    pub strength: f64,
    pub text: String,
}

// ── 综合输出 ──────────────────────────────────────────────────────────

/// 单周期分析结果（JSON 输出契约的核心）。
#[derive(Debug, Clone, Serialize)]
pub struct AnalysisOutput {
    pub schema_version: String,
    pub model_version: String,
    pub config_version: String,
    pub config_hash: Option<String>,
    pub enabled_features: Vec<String>,
    pub source: String,
    pub symbol: String,
    pub interval: String,
    pub time: i64,
    pub generated_at: i64,
    pub is_closed_kline: bool,
    pub data_quality: DataQuality,
    pub indicator_availability: IndicatorAvailability,
    pub raw_scores: Scores,
    pub smoothed_scores: Scores,
    pub score_momentum: ScoreMomentum,
    pub score_breakdown: ScoreBreakdown,
    pub state: MarketState,
    pub state_phase: StatePhase,
    pub state_transition: StateTransition,
    pub confidence_breakdown: ConfidenceBreakdown,
    pub risk_override: RiskOverride,
    pub risk_decision: RiskDecision,
    pub grid_plan: DisplayGridPlan,
    pub signals: Vec<Signal>,
}

/// 多周期分析结果。
#[derive(Debug, Clone, Serialize)]
pub struct MultiTfAnalysisOutput {
    pub schema_version: String,
    pub model_version: String,
    pub config_version: String,
    pub config_hash: Option<String>,
    pub enabled_features: Vec<String>,
    pub source: String,
    pub symbol: String,
    pub generated_at: i64,
    pub snapshots: Vec<TimeframeSnapshotRef>,
    pub merged_state: MarketState,
    pub merged_state_phase: StatePhase,
    pub risk_decision: RiskDecision,
    pub grid_plan: DisplayGridPlan,
    pub reasons: Vec<String>,
}

// ── 中间计算结果 ──────────────────────────────────────────────────────

/// 指标计算结果集，用于在评分模块和状态机之间传递。
#[derive(Debug, Clone)]
pub struct IndicatorResults {
    pub boll_upper: Vec<f64>,
    pub boll_mid: Vec<f64>,
    pub boll_lower: Vec<f64>,
    pub boll_bandwidth: Vec<f64>,
    pub percent_b: Vec<f64>,
    pub macd_dif: Vec<f64>,
    pub macd_dea: Vec<f64>,
    pub macd_hist: Vec<f64>,
    pub macd_golden_cross: Vec<bool>,
    pub macd_death_cross: Vec<bool>,
    pub atr: Vec<f64>,
    pub adx: Vec<f64>,
    pub plus_di: Vec<f64>,
    pub minus_di: Vec<f64>,
    pub rsi: Vec<f64>,
    pub ma20: Vec<f64>,
    pub ma60: Vec<f64>,
    pub ema20: Vec<f64>,
    pub ma_spread: Vec<f64>,
    pub ma20_slope: Vec<f64>,
    pub ema20_deviation: Vec<f64>,
    pub volume_ratio: Vec<f64>,
    pub donchian_upper: Vec<f64>,
    pub donchian_lower: Vec<f64>,
    /// 各指标是否在 last index 可用
    pub availability: IndicatorAvailability,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_interval_ms() {
        assert_eq!(parse_interval_ms("1m"), 60_000);
        assert_eq!(parse_interval_ms("5m"), 300_000);
        assert_eq!(parse_interval_ms("1h"), 3_600_000);
        assert_eq!(parse_interval_ms("4h"), 14_400_000);
        assert_eq!(parse_interval_ms("1d"), 86_400_000);
    }

    #[test]
    fn test_market_state_serde() {
        let state = MarketState::RangeGrid;
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(json, r#""range_grid""#);
        let back: MarketState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, MarketState::RangeGrid);
    }
}
