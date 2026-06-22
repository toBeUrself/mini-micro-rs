//! 六状态状态机与假突破/插针过滤器。
//!
//! 状态迁移优先级（由高到低）：
//! 1. RiskOverride / EmergencyStop → 覆盖执行动作，但不改变 MarketState 语义
//! 2. 数据质量严重不足 → Wait
//! 3. confirmed downtrend_risk
//! 4. confirmed down_break_warning
//! 5. confirmed uptrend_follow
//! 6. warning 状态
//! 7. range_grid
//! 8. wait

use crate::config::StateConfig;
use crate::models::{MarketState, StatePhase, StateContext, StateTransition, Scores, DataQuality};

/// 多空冲突的独立阈值（与 trend_candidate 解耦）。
/// spec 要求 up >= 70 && down >= 70 → wait。
const CONFLICT_THRESHOLD: f64 = 70.0;

/// 假突破价格回归阈值：作为窗口最高/最低价的比例。
/// 向上假突破：last_close < max_high * FAKE_OUT_HIGH_RATIO → 价格回归证据成立。
/// 向下假突破：last_close > min_low * FAKE_OUT_LOW_RATIO → 价格回归证据成立。
const FAKE_OUT_HIGH_RATIO: f64 = 0.98;
const FAKE_OUT_LOW_RATIO: f64 = 1.02;

/// 状态机推进主函数。
///
/// - `smoothed_scores`：经过平滑的三维评分
/// - `data_quality`：数据质量评估
/// - `indicator_ready`：核心指标是否就绪
/// - `ctx`：前序状态上下文（会被修改）
/// - `sc`：状态配置
/// - `enable_fake_breakout`：是否启用假突破过滤
/// - `klines_closed`：已闭合 K 线数组 [(open_time, open, high, low, close)]
/// - `volume_ratios`：最近窗口内的 volume_ratio 值（用于假突破成交量证据）
/// - `adx_values`：最近窗口内的 ADX 值（用于假突破趋势证据）
/// - `boll_upper`：BOLL 上轨最新值（用于插针过滤）
/// - `boll_lower`：BOLL 下轨最新值（用于插针过滤）
#[allow(clippy::too_many_arguments)]
pub fn advance_state(
    smoothed_scores: &Scores,
    data_quality: &DataQuality,
    indicator_ready: bool,
    ctx: &mut StateContext,
    sc: &StateConfig,
    enable_fake_breakout: bool,
    klines_closed: &[(i64, f64, f64, f64, f64)],
    volume_ratios: &[f64],
    adx_values: &[f64],
    boll_upper: Option<f64>,
    boll_lower: Option<f64>,
) -> StateTransition {
    let mut reasons: Vec<String> = Vec::new();

    // 同步 config 到 context
    ctx.required_confirm_bars = sc.confirm_bars;

    // ── P0：数据质量严重不足 ──────────────────────────────────────────
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

    // ── P0：指标不就绪 ──────────────────────────────────────────────
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

    // ── 冷却期推进 ──────────────────────────────────────────────────
    if ctx.cooldown_remaining_bars > 0 {
        ctx.cooldown_remaining_bars -= 1;
        if ctx.cooldown_remaining_bars > 0 {
            reasons.push(format!(
                "冷却中，剩余 {} bars",
                ctx.cooldown_remaining_bars
            ));
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
        // 冷却期结束，恢复到 Observing 阶段
        ctx.previous_state_phase = StatePhase::Observing;
    }

    let range = smoothed_scores.range_score;
    let up = smoothed_scores.up_score;
    let down = smoothed_scores.down_score;

    // ── 多空冲突处理（使用独立阈值 CONFLICT_THRESHOLD = 70）────────────
    if up >= CONFLICT_THRESHOLD && down >= CONFLICT_THRESHOLD {
        reasons.push(format!(
            "多空评分冲突: up={up:.0}, down={down:.0} → wait"
        ));
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

    // ── 确定候选状态 ──────────────────────────────────────────────────
    let candidate = determine_candidate_state(range, up, down, sc);
    reasons.push(format!("候选状态: {:?}", candidate));

    // ── 确认期逻辑 ──────────────────────────────────────────────────
    if Some(candidate) == ctx.candidate_state {
        ctx.candidate_bars += 1;
    } else {
        ctx.candidate_state = Some(candidate);
        ctx.candidate_bars = 1;
    }
    reasons.push(format!("候选计数: {}/{}", ctx.candidate_bars, sc.confirm_bars));

    // ── 候选状态评分下破退出线 → 重新评估而非强制 Wait ───────────────
    let candidate_invalid = match candidate {
        MarketState::RangeGrid => range < sc.range_exit,
        MarketState::UpBreakWarning | MarketState::UptrendFollow => up < sc.warning_exit,
        MarketState::DownBreakWarning | MarketState::DowntrendRisk => down < sc.warning_exit,
        MarketState::Wait => false,
    };
    if candidate_invalid {
        // 重新评估：可能 range 评分仍然满足，可从 trend warning 回退到 RangeGrid
        let fallback = determine_candidate_state(range, up, down, sc);
        reasons.push(format!(
            "候选状态评分跌破退出线，重新评估 → {:?}",
            fallback
        ));
        if fallback != candidate && fallback != MarketState::Wait {
            // 有合理的回退候选状态
            ctx.candidate_state = Some(fallback);
            ctx.candidate_bars = 1;
            let transition = StateTransition {
                previous_state: ctx.previous_state,
                candidate_state: Some(fallback),
                final_state: fallback,
                final_state_phase: StatePhase::Candidate,
                transition_type: "candidate_fallback".into(),
                candidate_bars: 1,
                cooldown_remaining_bars: ctx.cooldown_remaining_bars,
                reasons,
            };
            ctx.previous_state = fallback;
            ctx.previous_state_phase = StatePhase::Candidate;
            return transition;
        }
        // 无合适的回退状态 → Wait
        ctx.candidate_bars = 0;
        ctx.candidate_state = None;
        let transition = StateTransition {
            previous_state: ctx.previous_state,
            candidate_state: None,
            final_state: MarketState::Wait,
            final_state_phase: StatePhase::Observing,
            transition_type: "candidate_exit".into(),
            candidate_bars: 0,
            cooldown_remaining_bars: ctx.cooldown_remaining_bars,
            reasons,
        };
        ctx.previous_state = MarketState::Wait;
        ctx.previous_state_phase = StatePhase::Observing;
        return transition;
    }

    // ── 确认判断 ──────────────────────────────────────────────────────
    if ctx.candidate_bars >= sc.confirm_bars {
        // ── 假突破过滤（Phase 2） ──────────────────────────────────────
        if enable_fake_breakout {
            if let Some(override_state) = apply_fake_breakout_filter(
                candidate, klines_closed, volume_ratios, adx_values, sc,
            ) {
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

        // ── 插针过滤 ──────────────────────────────────────────────────
        if let (Some(upper), Some(lower)) = (boll_upper, boll_lower) {
            if let Some(filtered) = apply_wick_filter(
                candidate, klines_closed, upper, lower,
            ) {
                reasons.push(format!("插针过滤触发: → {:?}", filtered));
                let transition = StateTransition {
                    previous_state: ctx.previous_state,
                    candidate_state: Some(candidate),
                    final_state: filtered,
                    final_state_phase: StatePhase::Confirmed,
                    transition_type: "wick_filter_revert".into(),
                    candidate_bars: ctx.candidate_bars,
                    cooldown_remaining_bars: 0,
                    reasons: reasons.clone(),
                };
                ctx.previous_state = filtered;
                ctx.previous_state_phase = StatePhase::Confirmed;
                ctx.candidate_state = None;
                ctx.candidate_bars = 0;
                return transition;
            }
        }

        // ── 正常确认，设置冷却期 ──────────────────────────────────────
        // 只有从趋势状态退出时才设置止损冷却期
        let is_exiting_trend = matches!(
            ctx.previous_state,
            MarketState::DowntrendRisk | MarketState::UptrendFollow
        );
        let is_entering_trend = matches!(
            candidate,
            MarketState::DowntrendRisk | MarketState::UptrendFollow
        );

        // 退出趋势（从趋势状态切到非趋势）→ 止损冷却期
        // 进入趋势（从非趋势切到趋势）→ 不设冷却期，因为趋势确认本身就是正向切换
        if is_exiting_trend && !is_entering_trend {
            ctx.cooldown_remaining_bars = sc.cooldown_bars_after_stop_loss;
            reasons.push(format!(
                "退出趋势状态，设置止损冷却期 {} bars",
                sc.cooldown_bars_after_stop_loss
            ));
        } else if candidate != ctx.previous_state && !is_entering_trend {
            // 非趋势状态之间的切换 → 普通冷却期
            ctx.cooldown_remaining_bars = sc.cooldown_bars_after_exit;
            reasons.push(format!(
                "状态切换，设置冷却期 {} bars",
                sc.cooldown_bars_after_exit
            ));
        }

        reasons.push(format!("确认状态: {:?}", candidate));

        let transition = StateTransition {
            previous_state: ctx.previous_state,
            candidate_state: Some(candidate),
            final_state: candidate,
            final_state_phase: StatePhase::Confirmed,
            transition_type: "confirmed".into(),
            candidate_bars: ctx.candidate_bars,
            cooldown_remaining_bars: ctx.cooldown_remaining_bars,
            reasons: reasons.clone(),
        };

        ctx.previous_state = candidate;
        ctx.previous_state_phase = StatePhase::Confirmed;
        ctx.candidate_state = None;
        ctx.candidate_bars = 0;

        transition
    } else {
        // ── 处于候选期，保持前一状态或 candidate ──────────────────────
        let (final_state, phase) = if ctx.previous_state == MarketState::Wait {
            (candidate, StatePhase::Candidate)
        } else {
            (ctx.previous_state, StatePhase::Confirmed)
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
}

/// 根据评分确定候选状态。
fn determine_candidate_state(range: f64, up: f64, down: f64, sc: &StateConfig) -> MarketState {
    // downtrend_risk 优先（风险优先原则）
    if down >= sc.trend_confirm {
        return MarketState::DowntrendRisk;
    }
    if down >= sc.trend_candidate {
        return MarketState::DowntrendRisk;
    }
    if down >= sc.warning_enter {
        return MarketState::DownBreakWarning;
    }

    // uptrend_follow
    if up >= sc.trend_confirm {
        return MarketState::UptrendFollow;
    }
    if up >= sc.trend_candidate {
        return MarketState::UptrendFollow;
    }
    if up >= sc.warning_enter {
        return MarketState::UpBreakWarning;
    }

    // range_grid
    if range >= sc.range_enter {
        return MarketState::RangeGrid;
    }

    // default
    MarketState::Wait
}

/// 假突破过滤器（多证据联合判断）。
///
/// spec 要求至少两类以上证据同时成立：
/// 1. 价格证据：收盘价回归区间
/// 2. 成交量证据：volume_ratio < 阈值（缩量）
/// 3. 趋势证据：ADX 未继续上升
fn apply_fake_breakout_filter(
    candidate: MarketState,
    klines_closed: &[(i64, f64, f64, f64, f64)],
    volume_ratios: &[f64],
    adx_values: &[f64],
    sc: &StateConfig,
) -> Option<MarketState> {
    let window = sc.fake_breakout_window;
    if klines_closed.len() < window {
        return None;
    }

    let recent = &klines_closed[klines_closed.len() - window..];
    // 对齐 volume_ratios 和 adx_values 窗口
    let vr_start = volume_ratios.len().saturating_sub(window);
    let vr_window = &volume_ratios[vr_start..];
    let adx_start = adx_values.len().saturating_sub(window);
    let adx_window = &adx_values[adx_start..];

    match candidate {
        MarketState::UpBreakWarning | MarketState::UptrendFollow => {
            let highs: Vec<f64> = recent.iter().map(|(_, _, h, _, _)| *h).collect();
            let closes: Vec<f64> = recent.iter().map(|(_, _, _, _, c)| *c).collect();
            let max_high = highs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

            let mut evidence_count: usize = 0;

            // 证据1: 价格回归 — 最新收盘价低于窗口最高价的 FAKE_OUT_HIGH_RATIO
            let price_revert = closes.last().map_or(false, |&c| c < max_high * FAKE_OUT_HIGH_RATIO);
            if price_revert { evidence_count += 1; }

            // 证据2: 缩量 — 窗口内 volume_ratio 均值 < 突破确认阈值
            let vol_weak = if !vr_window.is_empty() {
                let avg_vr: f64 = vr_window.iter().filter(|v| v.is_finite()).sum::<f64>()
                    / vr_window.iter().filter(|v| v.is_finite()).count().max(1) as f64;
                avg_vr < sc.breakout_volume_confirm_threshold
            } else { false };
            if vol_weak { evidence_count += 1; }

            // 证据3: ADX 未继续上升
            let adx_not_rising = if adx_window.len() >= 2 {
                let first = adx_window[0];
                let last = adx_window[adx_window.len() - 1];
                first.is_finite() && last.is_finite() && last <= first
            } else { false };
            if adx_not_rising { evidence_count += 1; }

            if evidence_count >= 2 {
                return Some(MarketState::RangeGrid);
            }
        }
        MarketState::DownBreakWarning | MarketState::DowntrendRisk => {
            let lows: Vec<f64> = recent.iter().map(|(_, _, _, l, _)| *l).collect();
            let closes: Vec<f64> = recent.iter().map(|(_, _, _, _, c)| *c).collect();
            let min_low = lows.iter().cloned().fold(f64::INFINITY, f64::min);

            let mut evidence_count: usize = 0;

            // 证据1: 价格回归 — 最新收盘价高于窗口最低价的 FAKE_OUT_LOW_RATIO
            let price_revert = closes.last().map_or(false, |&c| c > min_low * FAKE_OUT_LOW_RATIO);
            if price_revert { evidence_count += 1; }

            // 证据2: 缩量
            let vol_weak = if !vr_window.is_empty() {
                let avg_vr: f64 = vr_window.iter().filter(|v| v.is_finite()).sum::<f64>()
                    / vr_window.iter().filter(|v| v.is_finite()).count().max(1) as f64;
                avg_vr < sc.breakout_volume_confirm_threshold
            } else { false };
            if vol_weak { evidence_count += 1; }

            // 证据3: ADX 未继续上升
            let adx_not_rising = if adx_window.len() >= 2 {
                let first = adx_window[0];
                let last = adx_window[adx_window.len() - 1];
                first.is_finite() && last.is_finite() && last <= first
            } else { false };
            if adx_not_rising { evidence_count += 1; }

            if evidence_count >= 2 {
                return Some(MarketState::RangeGrid);
            }
        }
        _ => {}
    }

    None
}

/// 插针/影线过滤器。
///
/// 仅 high/low 突破边界但 close 回到区间内 → 视为插针，不触发趋势确认。
/// - 向上插针：high > upper_boundary && close <= upper_boundary
/// - 向下插针：low < lower_boundary && close >= lower_boundary
fn apply_wick_filter(
    candidate: MarketState,
    klines_closed: &[(i64, f64, f64, f64, f64)],
    boll_upper: f64,
    boll_lower: f64,
) -> Option<MarketState> {
    if klines_closed.is_empty() {
        return None;
    }

    let &(_ot, _o, high, low, close) = klines_closed.last().unwrap();

    match candidate {
        MarketState::UpBreakWarning | MarketState::UptrendFollow => {
            // 向上插针：high突破上轨但close回到上轨下方
            if high > boll_upper && close <= boll_upper {
                return Some(MarketState::RangeGrid);
            }
        }
        MarketState::DownBreakWarning | MarketState::DowntrendRisk => {
            // 向下插针：low跌破下轨但close回到下轨上方
            if low < boll_lower && close >= boll_lower {
                return Some(MarketState::RangeGrid);
            }
        }
        _ => {}
    }

    None
}

/// 初始化状态上下文。
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

    fn empty_aux() -> (Vec<f64>, Vec<f64>) {
        (vec![], vec![])
    }

    #[test]
    fn test_no_data_returns_wait() {
        let scores = Scores { range_score: 0.0, up_score: 0.0, down_score: 0.0 };
        let dq = make_dq(false, 0.0);
        let sc = StateConfig::default();
        let mut ctx = new_state_context();
        let (vr, adx) = empty_aux();
        let transition = advance_state(&scores, &dq, false, &mut ctx, &sc, false, &[], &vr, &adx, None, None);
        assert_eq!(transition.final_state, MarketState::Wait);
    }

    #[test]
    fn test_range_grid_candidate() {
        let scores = Scores { range_score: 75.0, up_score: 20.0, down_score: 15.0 };
        let dq = make_dq(true, 1.0);
        let sc = StateConfig::default();
        let mut ctx = new_state_context();
        let (vr, adx) = empty_aux();
        let transition = advance_state(&scores, &dq, true, &mut ctx, &sc, false, &[], &vr, &adx, None, None);
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
        let (vr, adx) = empty_aux();

        for _ in 0..2 {
            advance_state(&scores, &dq, true, &mut ctx, &sc, false, &[], &vr, &adx, None, None);
        }
        assert_eq!(ctx.candidate_bars, 2);

        let transition = advance_state(&scores, &dq, true, &mut ctx, &sc, false, &[], &vr, &adx, None, None);
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
        let (vr, adx) = empty_aux();
        let transition = advance_state(&scores, &dq, true, &mut ctx, &sc, false, &[], &vr, &adx, None, None);
        assert_eq!(transition.final_state, MarketState::DowntrendRisk);
    }

    #[test]
    fn test_conflict_up_and_down_returns_wait() {
        let scores = Scores { range_score: 50.0, up_score: 75.0, down_score: 75.0 };
        let dq = make_dq(true, 1.0);
        let sc = StateConfig::default();
        let mut ctx = new_state_context();
        let (vr, adx) = empty_aux();
        let transition = advance_state(&scores, &dq, true, &mut ctx, &sc, false, &[], &vr, &adx, None, None);
        assert_eq!(transition.final_state, MarketState::Wait);
    }

    #[test]
    fn test_wick_filter_up() {
        // 向上插针：high > boll_upper, close <= boll_upper
        let klines = vec![(100, 100.0, 110.0, 99.0, 105.0)]; // high=110 > upper=108, close=105 <=108
        let result = apply_wick_filter(
            MarketState::UpBreakWarning,
            &klines,
            108.0,
            95.0,
        );
        assert_eq!(result, Some(MarketState::RangeGrid));
    }

    #[test]
    fn test_wick_filter_down() {
        // 向下插针：low < boll_lower, close >= boll_lower
        let klines = vec![(100, 100.0, 105.0, 93.0, 97.0)]; // low=93 < lower=95, close=97 >=95
        let result = apply_wick_filter(
            MarketState::DownBreakWarning,
            &klines,
            108.0,
            95.0,
        );
        assert_eq!(result, Some(MarketState::RangeGrid));
    }

    #[test]
    fn test_cooldown_applied_on_confirmed() {
        let scores = Scores { range_score: 75.0, up_score: 20.0, down_score: 15.0 };
        let dq = make_dq(true, 1.0);
        let sc = StateConfig::default();
        let mut ctx = new_state_context();
        // Set previous state to DowntrendRisk to trigger stop-loss cooldown
        ctx.previous_state = MarketState::DowntrendRisk;
        let (vr, adx) = empty_aux();

        // 3 bars to confirm (RangGrid from DowntrendRisk = exiting trend)
        for _ in 0..3 {
            advance_state(&scores, &dq, true, &mut ctx, &sc, false, &[], &vr, &adx, None, None);
        }
        // Should have set cooldown from exiting trend
        assert!(ctx.cooldown_remaining_bars > 0,
            "cooldown should be set when exiting trend state, got {}",
            ctx.cooldown_remaining_bars);
    }
}
