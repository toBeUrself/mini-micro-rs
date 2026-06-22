//! 置信度分解计算。
//!
//! 公式：
//! final_confidence = min(state_evidence, data_quality, indicator_availability, timeframe_alignment) * state_stability

use crate::models::{ConfidenceBreakdown, DataQuality, IndicatorAvailability, MarketState, StatePhase};

/// 评分差值映射到 evidence 的分母。
/// 例如 range_score 比 up/down 高 30 分 → evidence ≈ 1.0。
const EVIDENCE_DIVISOR: f64 = 30.0;

/// 指标不可用时的衰减底数：每个不可用字段将因子乘以该值。
const INDICATOR_POWER_BASE: f64 = 0.8;
/// 指标衰减的下限，避免因子降至 0。
const INDICATOR_POWER_MIN: f64 = 0.3;

/// Wait 状态的默认 state_evidence。
/// 无明确方向信号时使用此保守值。
const WAIT_EVIDENCE: f64 = 0.2;

/// 计算置信度分解。
///
/// - `state_evidence`：基于评分差值（状态信号多清晰）
/// - `data_quality`：数据质量评分
/// - `indicator_availability`：指标可用性
/// - `timeframe_alignment`：多周期一致性（单周期默认为 1.0）
/// - `state_stability`：状态稳定性（已确认则高，候选则低）
pub fn compute_confidence(
    raw_scores: &crate::models::Scores,
    data_quality: &DataQuality,
    indicator_avail: &IndicatorAvailability,
    state: MarketState,
    state_phase: StatePhase,
    timeframe_alignment: f64,
) -> ConfidenceBreakdown {
    // state_evidence: 评分差值越大越清晰
    let (range, up, down) = (raw_scores.range_score, raw_scores.up_score, raw_scores.down_score);
    let evidence = match state {
        MarketState::RangeGrid => {
            let margin = (range - up.max(down)).max(0.0);
            (margin / EVIDENCE_DIVISOR).clamp(0.0, 1.0)
        }
        MarketState::UpBreakWarning | MarketState::UptrendFollow => {
            let margin = (up - range.max(down)).max(0.0);
            (margin / EVIDENCE_DIVISOR).clamp(0.0, 1.0)
        }
        MarketState::DownBreakWarning | MarketState::DowntrendRisk => {
            let margin = (down - range.max(up)).max(0.0);
            (margin / EVIDENCE_DIVISOR).clamp(0.0, 1.0)
        }
        MarketState::Wait => WAIT_EVIDENCE,
    };

    let dq_factor = data_quality.quality_score.clamp(0.0, 1.0);

    let indicator_factor = if indicator_avail.ready {
        if indicator_avail.unavailable_fields.is_empty() {
            1.0
        } else {
            INDICATOR_POWER_BASE.powf(indicator_avail.unavailable_fields.len() as f64).max(INDICATOR_POWER_MIN)
        }
    } else {
        0.2
    };

    let tf_factor = timeframe_alignment.clamp(0.0, 1.0);

    let stability = match state_phase {
        StatePhase::Confirmed => 1.0,
        StatePhase::Candidate => 0.6,
        StatePhase::CoolingDown => 0.5,
        StatePhase::Observing => 0.3,
    };

    let final_conf = evidence.min(dq_factor).min(indicator_factor).min(tf_factor) * stability;

    ConfidenceBreakdown {
        state_evidence: evidence,
        data_quality: dq_factor,
        indicator_availability: indicator_factor,
        timeframe_alignment: tf_factor,
        state_stability: stability,
        final_confidence: final_conf.clamp(0.0, 1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_confidence_range_grid_confirmed() {
        let scores = crate::models::Scores { range_score: 75.0, up_score: 20.0, down_score: 15.0 };
        let dq = DataQuality {
            input_kline_count: 100, usable_closed_kline_count: 100,
            first_open_time: Some(0), last_open_time: Some(100_000),
            expected_interval_ms: 300_000, missing_kline_count: 0,
            missing_kline_ratio: 0.0, max_gap_bars: 0,
            gap_ranges: vec![], duplicate_kline_count: 0, out_of_order_count: 0,
            invalid_ohlcv_count: 0, has_gap: false, has_unclosed_kline: false,
            latest_kline_delay_ms: 0, warmup_satisfied: true, quality_score: 1.0,
            issues: vec![],
        };
        let ia = IndicatorAvailability {
            ready: true, min_required_bars: 60, warmup_bars: 100,
            unavailable_fields: vec![],
        };

        let cb = compute_confidence(
            &scores, &dq, &ia,
            MarketState::RangeGrid, StatePhase::Confirmed, 1.0,
        );
        assert!(cb.final_confidence > 0.7);
    }

    #[test]
    fn test_confidence_wait_state_low() {
        let scores = crate::models::Scores { range_score: 50.0, up_score: 40.0, down_score: 40.0 };
        let dq = DataQuality {
            input_kline_count: 100, usable_closed_kline_count: 100,
            first_open_time: Some(0), last_open_time: Some(100_000),
            expected_interval_ms: 300_000, missing_kline_count: 0,
            missing_kline_ratio: 0.0, max_gap_bars: 0,
            gap_ranges: vec![], duplicate_kline_count: 0, out_of_order_count: 0,
            invalid_ohlcv_count: 0, has_gap: false, has_unclosed_kline: false,
            latest_kline_delay_ms: 0, warmup_satisfied: true, quality_score: 1.0,
            issues: vec![],
        };
        let ia = IndicatorAvailability {
            ready: true, min_required_bars: 60, warmup_bars: 100,
            unavailable_fields: vec![],
        };

        let cb = compute_confidence(
            &scores, &dq, &ia,
            MarketState::Wait, StatePhase::Observing, 1.0,
        );
        assert!(cb.final_confidence < 0.5);
    }
}
