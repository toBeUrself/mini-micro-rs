//! JSON 输出契约构建。
//!
//! 确保 required 字段与 spec 一致：schema_version, model_version, config_version,
//! config_hash, enabled_features, source, symbol, interval, time, generated_at,
//! is_closed_kline, data_quality, indicator_availability, raw_scores, smoothed_scores,
//! score_momentum, score_breakdown, state, state_phase, state_transition,
//! confidence_breakdown, risk_override, risk_decision, grid_plan, signals.

use crate::config::KlinesToolsConfig;
use crate::models::*;

/// 构建单周期完整分析输出。
pub fn build_analysis_output(
    source: &str,
    symbol: &str,
    interval: &str,
    kline_time: i64,
    is_closed: bool,
    data_quality: &DataQuality,
    indicator_availability: &IndicatorAvailability,
    raw_scores: &Scores,
    smoothed_scores: &Scores,
    score_momentum: &ScoreMomentum,
    score_breakdown: &ScoreBreakdown,
    state: MarketState,
    state_phase: StatePhase,
    state_transition: &StateTransition,
    confidence_breakdown: &ConfidenceBreakdown,
    risk_override: RiskOverride,
    risk_decision: &RiskDecision,
    grid_plan: &DisplayGridPlan,
    signals: &[Signal],
    config: &KlinesToolsConfig,
) -> AnalysisOutput {
    let now_ms = chrono::Utc::now().timestamp_millis();

    AnalysisOutput {
        schema_version: "1.2".into(),
        model_version: "rule-v1".into(),
        config_version: "grid-analysis-v1.0.3".into(),
        config_hash: Some(config.config_hash()),
        enabled_features: config.enabled_features(),
        source: source.to_string(),
        symbol: symbol.to_string(),
        interval: interval.to_string(),
        time: kline_time,
        generated_at: now_ms,
        is_closed_kline: is_closed,
        data_quality: data_quality.clone(),
        indicator_availability: indicator_availability.clone(),
        raw_scores: raw_scores.clone(),
        smoothed_scores: smoothed_scores.clone(),
        score_momentum: score_momentum.clone(),
        score_breakdown: score_breakdown.clone(),
        state,
        state_phase,
        state_transition: state_transition.clone(),
        confidence_breakdown: confidence_breakdown.clone(),
        risk_override,
        risk_decision: risk_decision.clone(),
        grid_plan: grid_plan.clone(),
        signals: signals.to_vec(),
    }
}

/// 构建多周期分析输出。
pub fn build_multi_tf_output(
    source: &str,
    symbol: &str,
    snapshots: Vec<TimeframeSnapshotRef>,
    merged_state: MarketState,
    merged_state_phase: StatePhase,
    risk_decision: &RiskDecision,
    grid_plan: &DisplayGridPlan,
    reasons: Vec<String>,
    config: &KlinesToolsConfig,
) -> MultiTfAnalysisOutput {
    MultiTfAnalysisOutput {
        schema_version: "1.2".into(),
        model_version: "rule-v1".into(),
        config_version: "grid-analysis-v1.0.3".into(),
        config_hash: Some(config.config_hash()),
        enabled_features: config.enabled_features(),
        source: source.to_string(),
        symbol: symbol.to_string(),
        generated_at: chrono::Utc::now().timestamp_millis(),
        snapshots,
        merged_state,
        merged_state_phase,
        risk_decision: risk_decision.clone(),
        grid_plan: grid_plan.clone(),
        reasons,
    }
}
