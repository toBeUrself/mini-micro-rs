//! 六状态状态机与假突破过滤器。
//!
//! 状态迁移优先级（由高到低）：
//! 1. RiskOverride / EmergencyStop 由 risk layer 覆盖执行动作，不改写 MarketState
//! 2. 数据质量严重不足 → Wait
//! 3. confirmed downtrend_risk
//! 4. confirmed down_break_warning
//! 5. confirmed uptrend_follow
//! 6. warning 状态
//! 7. range_grid
//! 8. wait

use crate::config::StateConfig;
use crate::models::{DataQuality, MarketState, Scores, StateContext, StatePhase, StateTransition};

/// 状态机推进主函数。
pub fn advance_state(
    smoothed_scores: &Scores,
    data_quality: &DataQuality,
    indicator_ready: bool,
    ctx: &mut StateContext,
    sc: &StateConfig,
    enable_fake_breakout: bool,
    klines_closed: &[(i64, f64, f64, f64, f64)],
) -> StateTransition {
    let mut reasons: Vec<String> = Vec::new();

    if !data_quality.warmup_satisfied || data_quality.quality_score < 0.5 {
        let transition = StateTransition {
            previous_state: ctx.previous_state,
            candidate_state: None,
            final_state: MarketState::Wait,
            final_state_phase: StatePhase::Confirmed,
            transition_type: "data_quality_block".into(),
            candidate_bars: 0,
            cooldown_remaining_bars: 0,
            reasons: vec!["数据质量不足或 warmup 不满足".into()],
        };
        ctx.previous_state = MarketState::Wait;
        ctx.previous_state_phase = StatePhase::Confirmed;
        ctx.candidate_state = None;
        ctx.candidate_bars = 0;
        ctx.cooldown_remaining_bars = 0;
        return transition;
    }

    if !indicator_ready {
        let transition = StateTransition {
            previous_state: ctx.previous_state,
            candidate_state: None,
            final_state: MarketState::Wait,
            final_state_phase: StatePhase::Observing,
            transition_type: "indicator_not_ready".into(),
            candidate_bars: 0,
            cooldown_remaining_bars: 0,
            reasons: vec!["核心指标 warmup 不满足".into()],
        };
        ctx.previous_state = MarketState::Wait;
        ctx.previous_state_phase = StatePhase::Observing;
        ctx.candidate_state = None;
        ctx.candidate_bars = 0;
        return transition;
    }

    if ctx.cooldown_remaining_bars > 0 {
        ctx.cooldown_remaining_bars -= 1;
        if ctx.cooldown_remaining_bars > 0 {
            reasons.push(format!("冷却中，剩余 {} bars", ctx.cooldown_remaining_bars));
            return StateTransition {
                previous_state: ctx.previous_state,
                candidate_state: None,
                final_state: MarketState::Wait,
                final_state_phase: StatePhase::CoolingDown,
                transition_type: "cooldown".into(),
                candidate_bars: 0,
                cooldown_remaining_bars: ctx.cooldown_remaining_bars,
                reasons,
            };
        }
    }

    let range = smoothed_scores.range_score;
    let up = smoothed_scores.up_score;
    let down = smoothed_scores.down_score;

    if up >= sc.trend_candidate && down >= sc.trend_candidate {
        reasons.push(format!("多空评分冲突: up={up:.0}, down={down:.0} → wait"));
        let transition = StateTransition {
            previous_state: ctx.previous_state,
            candidate_state: None,
            final_state: MarketState::Wait,
            final_state_phase: StatePhase::Confirmed,
            transition_type: "conflict".into(),
            candidate_bars: 0,
            cooldown_remaining_bars: 0,
            reasons: reasons.clone(),
        };
        ctx.previous_state = MarketState::Wait;
        ctx.previous_state_phase = StatePhase::Confirmed;
        ctx.candidate_state = None;
        ctx.candidate_bars = 0;
        return transition;
    }

    let candidate = determine_candidate_state(range, up, down, sc);
    reasons.push(format!("候选状态: {:?}", candidate));

    if Some(candidate) == ctx.candidate_state {
        ctx.candidate_bars += 1;
    } else {
        ctx.candidate_state = Some(candidate);
        ctx.candidate_bars = 1;
    }
    reasons.push(format!("候选计数: {}/{}", ctx.candidate_bars, sc.confirm_bars));

    let candidate_invalid = match candidate {
        MarketState::RangeGrid => range < sc.range_exit,
        MarketState::UpBreakWarning | MarketState::UptrendFollow => up < sc.warning_exit,
        MarketState::DownBreakWarning | MarketState::DowntrendRisk => down < sc.warning_exit,
        MarketState::Wait => false,
    };
    if candidate_invalid {
        ctx.candidate_bars = 0;
        ctx.candidate_state = None;
        reasons.push("候选状态评分跌破退出线，清理候选".into());
        let transition = StateTransition {
            previous_state: ctx.previous_state,
            candidate_state: None,
            final_state: MarketState::Wait,
            final_state_phase: StatePhase::Observing,
            transition_type: "candidate_exit".into(),
            candidate_bars: 0,
            cooldown_remaining_bars: 0,
            reasons,
        };
        ctx.previous_state = MarketState::Wait;
        ctx.previous_state_phase = StatePhase::Observing;
        return transition;
    }

    if ctx.candidate_bars >= sc.confirm_bars {
        if enable_fake_breakout {
            if let Some(override_state) = apply_fake_breakout_filter(candidate, klines_closed, sc) {
                reasons.push(format!("假突破过滤触发: → {:?}", override_state));
                let transition = StateTransition {
                    previous_state: ctx.previous_state,
                    candidate_state: Some(candidate),
                    final_state: override_state,
                    final_state_phase: StatePhase::Confirmed,
                    transition_type: "fake_breakout_revert".into(),
                    candidate_bars: ctx.candidate_bars,
                    cooldown_remaining_bars: 0,
                    reasons: reasons.clone(),
                };
                ctx.previous_state = override_state;
                ctx.previous_state_phase = StatePhase::Confirmed;
                ctx.candidate_state = None;
                ctx.candidate_bars = 0;
                return transition;
            }
        }

        reasons.push(format!("确认状态: {:?}", candidate));
        let transition = StateTransition {
            previous_state: ctx.previous_state,
            candidate_state: Some(candidate),
            final_state: candidate,
            final_state_phase: StatePhase::Confirmed,
            transition_type: "confirmed".into(),
            candidate_bars: ctx.candidate_bars,
            cooldown_remaining_bars: 0,
            reasons: reasons.clone(),
        };

        ctx.previous_state = candidate;
        ctx.previous_state_phase = StatePhase::Confirmed;
        ctx.previous_state_since = klines_closed.last().map(|k| k.0).unwrap_or(ctx.previous_state_since);
        ctx.candidate_state = None;
        ctx.candidate_bars = 0;
        return transition;
    }

    // 候选未确认：如果已有 confirmed 状态，则保持旧 confirmed 状态；否则输出 candidate。
    let (final_state, phase) = if ctx.previous_state_phase == StatePhase::Confirmed
        && ctx.previous_state != MarketState::Wait
        && ctx.previous_state != candidate
    {
        (ctx.previous_state, StatePhase::Confirmed)
    } else {
        (candidate, StatePhase::Candidate)
    };

    let transition = StateTransition {
        previous_state: ctx.previous_state,
        candidate_state: Some(candidate),
        final_state,
        final_state_phase: phase,
        transition_type: "candidate".into(),
        candidate_bars: ctx.candidate_bars,
        cooldown_remaining_bars: ctx.cooldown_remaining_bars,
        reasons,
    };

    ctx.previous_state = final_state;
    ctx.previous_state_phase = phase;
    transition
}

fn determine_candidate_state(range: f64, up: f64, down: f64, sc: &StateConfig) -> MarketState {
    if down >= sc.trend_candidate {
        return MarketState::DowntrendRisk;
    }
    if down >= sc.warning_enter {
        return MarketState::DownBreakWarning;
    }
    if up >= sc.trend_candidate {
        return MarketState::UptrendFollow;
    }
    if up >= sc.warning_enter {
        return MarketState::UpBreakWarning;
    }
    if range >= sc.range_enter && up < sc.warning_enter && down < sc.warning_enter {
        return MarketState::RangeGrid;
    }
    MarketState::Wait
}

fn apply_fake_breakout_filter(
    candidate: MarketState,
    klines_closed: &[(i64, f64, f64, f64, f64)],
    sc: &StateConfig,
) -> Option<MarketState> {
    let window = sc.fake_breakout_window;
    if klines_closed.len() < window || window == 0 {
        return None;
    }
    let recent = &klines_closed[klines_closed.len() - window..];

    match candidate {
        MarketState::UpBreakWarning | MarketState::UptrendFollow => {
            let max_high = recent.iter().map(|(_, _, h, _, _)| *h).fold(f64::NEG_INFINITY, f64::max);
            if let Some(last_close) = recent.last().map(|(_, _, _, _, c)| *c) {
                if max_high.is_finite() && last_close < max_high * 0.98 {
                    return Some(MarketState::RangeGrid);
                }
            }
        }
        MarketState::DownBreakWarning | MarketState::DowntrendRisk => {
            let min_low = recent.iter().map(|(_, _, _, l, _)| *l).fold(f64::INFINITY, f64::min);
            if let Some(last_close) = recent.last().map(|(_, _, _, _, c)| *c) {
                if min_low.is_finite() && last_close > min_low * 1.02 {
                    return Some(MarketState::RangeGrid);
                }
            }
        }
        _ => {}
    }

    None
}

pub fn new_state_context() -> StateContext {
    StateContext::default()
}

/// 当触发止损时，设置冷却期。
pub fn trigger_stop_loss_cooldown(ctx: &mut StateContext, cooldown_bars: usize) {
    ctx.cooldown_remaining_bars = cooldown_bars;
    ctx.candidate_state = None;
    ctx.candidate_bars = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_dq(warmup: bool, score: f64) -> DataQuality {
        DataQuality {
            input_kline_count: 100,
            usable_closed_kline_count: 100,
            first_open_time: Some(0),
            last_open_time: Some(100_000),
            expected_interval_ms: 300_000,
            missing_kline_count: 0,
            missing_kline_ratio: 0.0,
            max_gap_bars: 0,
            gap_ranges: vec![],
            duplicate_kline_count: 0,
            out_of_order_count: 0,
            invalid_ohlcv_count: 0,
            has_gap: false,
            has_unclosed_kline: false,
            latest_kline_delay_ms: 0,
            warmup_satisfied: warmup,
            quality_score: score,
            issues: vec![],
        }
    }

    #[test]
    fn test_no_data_returns_wait() {
        let scores = Scores { range_score: 0.0, up_score: 0.0, down_score: 0.0 };
        let dq = make_dq(false, 0.0);
        let sc = StateConfig::default();
        let mut ctx = new_state_context();
        let transition = advance_state(&scores, &dq, false, &mut ctx, &sc, false, &[]);
        assert_eq!(transition.final_state, MarketState::Wait);
    }

    #[test]
    fn test_range_grid_candidate() {
        let scores = Scores { range_score: 75.0, up_score: 20.0, down_score: 15.0 };
        let dq = make_dq(true, 1.0);
        let sc = StateConfig::default();
        let mut ctx = new_state_context();
        let transition = advance_state(&scores, &dq, true, &mut ctx, &sc, false, &[]);
        assert_eq!(transition.final_state, MarketState::RangeGrid);
        assert_eq!(transition.final_state_phase, StatePhase::Candidate);
        assert_eq!(ctx.candidate_state, Some(MarketState::RangeGrid));
        assert_eq!(ctx.candidate_bars, 1);
    }

    #[test]
    fn test_range_grid_confirmed_after_bars() {
        let scores = Scores { range_score: 75.0, up_score: 20.0, down_score: 15.0 };
        let dq = make_dq(true, 1.0);
        let sc = StateConfig::default();
        let mut ctx = new_state_context();
        for _ in 0..2 {
            let t = advance_state(&scores, &dq, true, &mut ctx, &sc, false, &[]);
            assert_eq!(t.final_state_phase, StatePhase::Candidate);
        }
        let transition = advance_state(&scores, &dq, true, &mut ctx, &sc, false, &[]);
        assert_eq!(transition.final_state, MarketState::RangeGrid);
        assert_eq!(transition.final_state_phase, StatePhase::Confirmed);
        assert_eq!(transition.transition_type, "confirmed");
    }

    #[test]
    fn test_downtrend_risk_trumps_range() {
        let scores = Scores { range_score: 75.0, up_score: 20.0, down_score: 85.0 };
        let dq = make_dq(true, 1.0);
        let sc = StateConfig::default();
        let mut ctx = new_state_context();
        let transition = advance_state(&scores, &dq, true, &mut ctx, &sc, false, &[]);
        assert_eq!(transition.final_state, MarketState::DowntrendRisk);
        assert_eq!(transition.final_state_phase, StatePhase::Candidate);
    }

    #[test]
    fn test_conflict_up_and_down_returns_wait() {
        let scores = Scores { range_score: 50.0, up_score: 75.0, down_score: 75.0 };
        let dq = make_dq(true, 1.0);
        let sc = StateConfig::default();
        let mut ctx = new_state_context();
        let transition = advance_state(&scores, &dq, true, &mut ctx, &sc, false, &[]);
        assert_eq!(transition.final_state, MarketState::Wait);
    }
}
