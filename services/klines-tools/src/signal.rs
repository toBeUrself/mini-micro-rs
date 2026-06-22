//! 信号生成：前端展示和复盘标记。

use crate::models::{
    MarketState, StatePhase, StateTransition, Scores, Signal, SignalType,
};

/// 根据状态迁移生成对应信号。
pub fn generate_signals(
    transition: &StateTransition,
    raw_scores: &Scores,
    latest_price: f64,
    latest_time: i64,
) -> Vec<Signal> {
    let mut signals = Vec::new();

    // 根据迁移类型生成信号
    match transition.transition_type.as_str() {
        "confirmed" => {
            match transition.final_state {
                MarketState::RangeGrid => {
                    signals.push(Signal {
                        time: latest_time,
                        price: latest_price,
                        signal_type: SignalType::ResumeGrid,
                        strength: (raw_scores.range_score / 100.0).clamp(0.0, 1.0),
                        text: "震荡条件确认，恢复网格".into(),
                    });
                }
                MarketState::UpBreakWarning => {
                    signals.push(Signal {
                        time: latest_time,
                        price: latest_price,
                        signal_type: SignalType::UpBreakWarning,
                        strength: (raw_scores.up_score / 100.0).clamp(0.0, 1.0),
                        text: "上涨突破预警，减少卖出".into(),
                    });
                }
                MarketState::UptrendFollow => {
                    signals.push(Signal {
                        time: latest_time,
                        price: latest_price,
                        signal_type: SignalType::PauseGrid,
                        strength: (raw_scores.up_score / 100.0).clamp(0.0, 1.0),
                        text: "上涨趋势确认，关闭普通网格".into(),
                    });
                    signals.push(Signal {
                        time: latest_time,
                        price: latest_price,
                        signal_type: SignalType::MoveGridUp,
                        strength: 0.8,
                        text: "建议上移网格或切换趋势跟随".into(),
                    });
                }
                MarketState::DownBreakWarning => {
                    signals.push(Signal {
                        time: latest_time,
                        price: latest_price,
                        signal_type: SignalType::DownBreakWarning,
                        strength: (raw_scores.down_score / 100.0).clamp(0.0, 1.0),
                        text: "下跌破位预警，暂停新增买单".into(),
                    });
                }
                MarketState::DowntrendRisk => {
                    signals.push(Signal {
                        time: latest_time,
                        price: latest_price,
                        signal_type: SignalType::PauseGrid,
                        strength: 1.0,
                        text: "下跌风险确认，关闭网格".into(),
                    });
                    signals.push(Signal {
                        time: latest_time,
                        price: latest_price,
                        signal_type: SignalType::RiskReduce,
                        strength: (raw_scores.down_score / 100.0).clamp(0.0, 1.0),
                        text: "建议减仓或止损".into(),
                    });
                }
                _ => {}
            }
        }
        "fake_breakout_revert" => {
            signals.push(Signal {
                time: latest_time,
                price: latest_price,
                signal_type: SignalType::ResumeGrid,
                strength: 0.5,
                text: "假突破回撤，恢复震荡".into(),
            });
        }
        "conflict" => {
            signals.push(Signal {
                time: latest_time,
                price: latest_price,
                signal_type: SignalType::PauseGrid,
                strength: 0.7,
                text: "多空评分冲突，暂停网格".into(),
            });
        }
        _ => {}
    }

    // 网格买卖观察信号（仅在 RangeGrid 时）
    if transition.final_state == MarketState::RangeGrid
        && transition.final_state_phase == StatePhase::Confirmed
    {
        signals.push(Signal {
            time: latest_time,
            price: latest_price,
            signal_type: SignalType::GridBuyWatch,
            strength: 0.5,
            text: "观察买入触发".into(),
        });
        signals.push(Signal {
            time: latest_time,
            price: latest_price,
            signal_type: SignalType::GridSellWatch,
            strength: 0.5,
            text: "观察卖出触发".into(),
        });
    }

    signals
}
