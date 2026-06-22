//! 多周期时间对齐与合并决策矩阵。

use crate::models::{MarketState, StatePhase, TimeframeSnapshotRef};

/// 多周期合并决策。
///
/// 时间对齐规则：
/// - 所有状态确认只使用各周期最新已闭合 K 线。
/// - lower timeframe 只能引用 close_time <= T 的 higher/middle 分析结果。
/// - 如果传入 lower，则以 lower.close_time 为 anchor 校验 higher/middle。
///
/// 返回合并后的状态和决策。
pub fn merge_multi_timeframe(
    higher: Option<&TimeframeSnapshotRef>,
    middle: &TimeframeSnapshotRef,
    lower: Option<&TimeframeSnapshotRef>,
) -> (MarketState, StatePhase, Vec<String>) {
    let mut reasons: Vec<String> = Vec::new();

    if let Some(l) = lower {
        if middle.close_time > l.close_time {
            reasons.push(format!(
                "middle({}) close_time {} is newer than lower anchor {} → wait",
                middle.interval, middle.close_time, l.close_time
            ));
            return (MarketState::Wait, StatePhase::Observing, reasons);
        }
        if let Some(h) = higher {
            if h.close_time > l.close_time {
                reasons.push(format!(
                    "higher({}) close_time {} is newer than lower anchor {} → wait",
                    h.interval, h.close_time, l.close_time
                ));
                return (MarketState::Wait, StatePhase::Observing, reasons);
            }
        }
    }

    // P0: higher timeframe 已确认下跌风险 → hard block
    if let Some(h) = higher {
        if h.state == MarketState::DowntrendRisk && h.state_phase == StatePhase::Confirmed {
            reasons.push(format!(
                "higher({}) confirmed downtrend_risk → hard_block",
                h.interval
            ));
            return (MarketState::DowntrendRisk, StatePhase::Confirmed, reasons);
        }
        if h.state == MarketState::DownBreakWarning && h.state_phase == StatePhase::Confirmed {
            reasons.push(format!(
                "higher({}) confirmed down_break_warning → soft_block",
                h.interval
            ));
            return (MarketState::DownBreakWarning, StatePhase::Confirmed, reasons);
        }
    }

    // P1: middle timeframe 下跌风险
    if middle.state == MarketState::DowntrendRisk && middle.state_phase == StatePhase::Confirmed {
        reasons.push(format!(
            "middle({}) confirmed downtrend_risk → hard_block",
            middle.interval
        ));
        return (MarketState::DowntrendRisk, StatePhase::Confirmed, reasons);
    }

    if middle.state == MarketState::DownBreakWarning && middle.state_phase == StatePhase::Confirmed {
        reasons.push(format!(
            "middle({}) confirmed down_break_warning → 暂停新增买单",
            middle.interval
        ));
        return (MarketState::DownBreakWarning, StatePhase::Confirmed, reasons);
    }

    // P2: higher uptrend + middle range → only trend follow
    if let Some(h) = higher {
        if h.state == MarketState::UptrendFollow
            && h.state_phase == StatePhase::Confirmed
            && middle.state == MarketState::RangeGrid
        {
            reasons.push("higher uptrend + middle range → 只允许趋势跟随".into());
            return (MarketState::UptrendFollow, StatePhase::Confirmed, reasons);
        }
    }

    // P3: lower warnings
    if let Some(l) = lower {
        if l.state == MarketState::DownBreakWarning {
            reasons.push("lower down_break_warning → 不新增买单".into());
            return (MarketState::DownBreakWarning, StatePhase::Candidate, reasons);
        }
        if l.state == MarketState::UpBreakWarning {
            reasons.push("lower up_break_warning → 减少卖出".into());
            return (MarketState::UpBreakWarning, StatePhase::Candidate, reasons);
        }
    }

    // Default: use middle's state
    if middle.state == MarketState::RangeGrid && middle.state_phase == StatePhase::Confirmed {
        reasons.push("middle range_grid confirmed → 允许普通网格".into());
        (MarketState::RangeGrid, StatePhase::Confirmed, reasons)
    } else if middle.state_phase == StatePhase::Candidate {
        reasons.push(format!("middle state {:?} is candidate → wait", middle.state));
        (MarketState::Wait, StatePhase::Observing, reasons)
    } else {
        reasons.push(format!("middle state {:?} → wait", middle.state));
        (MarketState::Wait, StatePhase::Observing, reasons)
    }
}

/// 获取多周期默认配置的周期字符串列表。
pub fn get_default_intervals() -> Vec<String> {
    vec!["4h".into(), "30m".into(), "5m".into()]
}
