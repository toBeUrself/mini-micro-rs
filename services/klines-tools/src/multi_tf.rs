//! 多周期时间对齐与合并决策矩阵。
//!
//! SPEC 15.3 时间对齐规则：
//! - 所有状态确认只使用各周期最新已闭合 K 线。
//! - lower timeframe 只能引用 close_time <= T 的 higher/middle 分析结果。
//! - 不得用尚未闭合的 higher timeframe K线参与 lower timeframe 状态确认。
//! - 如果 higher timeframe 最新已闭合结果延迟超过配置阈值，则降低 confidence 或进入 wait。

use crate::models::{MarketState, StatePhase, TimeframeSnapshotRef};

/// 多周期合并决策。
///
/// 返回 (merged_state, merged_phase, reasons, confidence_modifier)。
/// confidence_modifier 是时间对齐修正系数（0.0~1.0），1.0 表示对齐完美。
pub fn merge_multi_timeframe(
    higher: Option<&TimeframeSnapshotRef>,
    middle: &TimeframeSnapshotRef,
    lower: Option<&TimeframeSnapshotRef>,
) -> (MarketState, StatePhase, Vec<String>, f64) {
    let mut reasons: Vec<String> = Vec::new();
    let mut confidence_modifier: f64 = 1.0;

    // ── 时间对齐校验（CRITICAL: SPEC 15.3） ────────────────────────────
    // middle 引用 higher 时，必须校验 higher.close_time <= middle.open_time
    if let Some(h) = higher {
        if !h.is_closed {
            reasons.push(format!(
                "higher({}) 未闭合K线，不得参与状态确认 → 降低confidence",
                h.interval
            ));
            confidence_modifier *= 0.5;
        }
        // 如果 higher 的 close_time 晚于 middle 的 open_time，说明 higher 还没走完就用了
        if h.close_time > middle.open_time && h.is_closed {
            reasons.push(format!(
                "higher({}) close_time({}) > middle open_time({}) → 时间对齐警告",
                h.interval, h.close_time, middle.open_time
            ));
            confidence_modifier *= 0.7;
        }
    }

    // lower 引用 middle 时，校验 middle.close_time <= lower.open_time
    if let Some(l) = lower {
        if !middle.is_closed {
            reasons.push(format!(
                "middle({}) 未闭合K线，不得参与 lower timeframe 确认 → 降低confidence",
                middle.interval
            ));
            confidence_modifier *= 0.5;
        }
        if middle.close_time > l.open_time && middle.is_closed {
            reasons.push(format!(
                "middle({}) close_time({}) > lower open_time({}) → 时间对齐警告",
                middle.interval, middle.close_time, l.open_time
            ));
            confidence_modifier *= 0.7;
        }
        if !l.is_closed {
            reasons.push(format!(
                "lower({}) 未闭合K线，仅用于观察 → 降低confidence",
                l.interval
            ));
            confidence_modifier *= 0.7;
        }
    }

    if confidence_modifier < 0.3 {
        reasons.push("时间对齐严重偏离，进入 wait".into());
        return (MarketState::Wait, StatePhase::Observing, reasons, confidence_modifier);
    }

    // ── P0: higher timeframe 已确认下跌风险 → hard block ────────────────
    if let Some(h) = higher {
        if h.is_closed && h.state == MarketState::DowntrendRisk && h.state_phase == StatePhase::Confirmed {
            reasons.push(format!(
                "higher({}) confirmed downtrend_risk → hard_block",
                h.interval
            ));
            return (MarketState::DowntrendRisk, StatePhase::Confirmed, reasons, confidence_modifier);
        }
        if h.is_closed && h.state == MarketState::DownBreakWarning && h.state_phase == StatePhase::Confirmed {
            reasons.push(format!(
                "higher({}) confirmed down_break_warning → soft_block",
                h.interval
            ));
        }
        if h.is_closed && h.state == MarketState::UptrendFollow && h.state_phase == StatePhase::Confirmed {
            reasons.push(format!(
                "higher({}) confirmed uptrend → only uptrend_follow allowed",
                h.interval
            ));
        }
    }

    // ── P1: middle timeframe 下跌风险 ──────────────────────────────────
    if middle.is_closed && middle.state == MarketState::DowntrendRisk && middle.state_phase == StatePhase::Confirmed {
        reasons.push(format!(
            "middle({}) confirmed downtrend_risk → hard_block",
            middle.interval
        ));
        return (MarketState::DowntrendRisk, StatePhase::Confirmed, reasons, confidence_modifier);
    }

    if middle.is_closed && middle.state == MarketState::DownBreakWarning && middle.state_phase == StatePhase::Confirmed {
        reasons.push(format!(
            "middle({}) confirmed down_break_warning → 暂停新增买单",
            middle.interval
        ));
        return (MarketState::DownBreakWarning, StatePhase::Confirmed, reasons, confidence_modifier);
    }

    // ── P2: higher uptrend → only trend follow ─────────────────────────
    if let Some(h) = higher {
        if h.is_closed && h.state == MarketState::UptrendFollow && h.state_phase == StatePhase::Confirmed
            && middle.state == MarketState::RangeGrid
        {
            reasons.push("higher uptrend + middle range → 只允许趋势跟随".into());
            return (MarketState::UptrendFollow, StatePhase::Confirmed, reasons, confidence_modifier);
        }
    }

    // ── P3: lower warnings ─────────────────────────────────────────────
    if let Some(l) = lower {
        if l.state == MarketState::DownBreakWarning {
            reasons.push("lower down_break_warning → 不新增买单".into());
            return (MarketState::DownBreakWarning, StatePhase::Candidate, reasons, confidence_modifier);
        }
        if l.state == MarketState::UpBreakWarning {
            reasons.push("lower up_break_warning → 减少卖出".into());
            return (MarketState::UpBreakWarning, StatePhase::Candidate, reasons, confidence_modifier);
        }
    }

    // ── Default: use middle's state ────────────────────────────────────
    if middle.state == MarketState::RangeGrid && middle.state_phase == StatePhase::Confirmed {
        reasons.push("middle range_grid confirmed → 允许普通网格".into());
        (MarketState::RangeGrid, StatePhase::Confirmed, reasons, confidence_modifier)
    } else {
        reasons.push(format!("middle state {:?} → wait", middle.state));
        (MarketState::Wait, StatePhase::Observing, reasons, confidence_modifier)
    }
}

/// 获取多周期默认配置的周期字符串列表。
pub fn get_default_intervals() -> Vec<String> {
    vec!["4h".into(), "30m".into(), "5m".into()]
}
