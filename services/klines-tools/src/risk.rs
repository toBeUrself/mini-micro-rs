//! 风险决策层：RiskOverride → RiskDecision。
//!
//! 市场状态只表达行情，RiskOverride 和责任动作不混入 MarketState。

use crate::config::RiskConfig;
use crate::models::{
    MarketState, RiskOverride, RiskLevel, RiskDecision, AllowedGridMode,
    OrderPermission, PositionAction, DataQuality, PortfolioRiskInput,
};

/// 从 RiskOverride 生成执行层 RiskDecision。
pub fn build_risk_decision(
    market_state: MarketState,
    risk_override: RiskOverride,
    data_quality: &DataQuality,
    portfolio: Option<&PortfolioRiskInput>,
    rc: &RiskConfig,
    _enable_exchange_constraints: bool,
) -> RiskDecision {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let ttl = 300_000; // 5 min
    let mut reasons: Vec<String> = Vec::new();

    // P0: RiskOverride 优先
    let (risk_level, order_perm, pos_action, allowed_modes) = match risk_override {
        RiskOverride::GlobalHardStop => {
            reasons.push("全局硬止损".into());
            (
                RiskLevel::EmergencyStop,
                OrderPermission::None,
                PositionAction::StopLoss,
                vec![],
            )
        }
        RiskOverride::DataQualityBlock => {
            reasons.push("数据质量阻断".into());
            (
                RiskLevel::HardBlock,
                OrderPermission::ReadOnly,
                PositionAction::Hold,
                vec![],
            )
        }
        RiskOverride::ManualBlock => {
            reasons.push("人工阻断".into());
            (
                RiskLevel::HardBlock,
                OrderPermission::ReadOnly,
                PositionAction::ManualReview,
                vec![],
            )
        }
        RiskOverride::IndicatorUnavailableBlock => {
            reasons.push("核心指标不可用".into());
            (
                RiskLevel::HardBlock,
                OrderPermission::ReadOnly,
                PositionAction::Hold,
                vec![],
            )
        }
        RiskOverride::ExchangeConstraintBlock => {
            reasons.push("交易所约束不满足".into());
            (
                RiskLevel::SoftBlock,
                OrderPermission::ReadOnly,
                PositionAction::Hold,
                vec![],
            )
        }
        RiskOverride::None => {
            // 从 MarketState 推导默认动作
            default_risk_from_state(market_state, data_quality, portfolio, rc, &mut reasons)
        }
    };

    let reduce_ratio = if matches!(pos_action, PositionAction::ReduceByRatio) {
        Some(rc.default_reduce_position_ratio)
    } else {
        None
    };

    RiskDecision {
        risk_level,
        risk_override,
        allowed_grid_modes: allowed_modes,
        order_permission: order_perm,
        position_action: pos_action,
        reduce_position_ratio: reduce_ratio,
        reduce_reference: if reduce_ratio.is_some() {
            Some("total_position".into())
        } else {
            None
        },
        require_manual_confirm: matches!(risk_level, RiskLevel::EmergencyStop | RiskLevel::HardBlock),
        action_ttl_ms: ttl,
        expire_at: now_ms + ttl,
        reasons,
    }
}

/// 从 MarketState 推导默认风险决策。
fn default_risk_from_state(
    state: MarketState,
    dq: &DataQuality,
    portfolio: Option<&PortfolioRiskInput>,
    rc: &RiskConfig,
    reasons: &mut Vec<String>,
) -> (RiskLevel, OrderPermission, PositionAction, Vec<AllowedGridMode>) {
    match state {
        MarketState::Wait => {
            reasons.push("方向不清晰或等待中".into());
            (RiskLevel::Advisory, OrderPermission::ReadOnly, PositionAction::Hold, vec![])
        }
        MarketState::RangeGrid => {
            // 检查数据质量
            if dq.quality_score < 0.7 {
                reasons.push(format!("数据质量过低({:.2})，不允许新开网格", dq.quality_score));
                return (RiskLevel::SoftBlock, OrderPermission::ReadOnly, PositionAction::Hold, vec![]);
            }
            // 检查是否已触发账户级风险
            if let Some(pf) = portfolio {
                if let Some(dd) = pf.max_equity_drawdown {
                    if dd > rc.max_drawdown {
                        reasons.push(format!("账户回撤({:.2}%)超过阈值", dd * 100.0));
                        return (RiskLevel::SoftBlock, OrderPermission::ReduceOnly, PositionAction::ReduceByRatio, vec![]);
                    }
                }
            }
            reasons.push("震荡条件成立，允许普通震荡网格".into());
            (
                RiskLevel::Advisory,
                OrderPermission::NewOrdersAllowed,
                PositionAction::Hold,
                vec![AllowedGridMode::RangeGrid],
            )
        }
        MarketState::UpBreakWarning => {
            reasons.push("上涨突破预警，减少卖出".into());
            (
                RiskLevel::Advisory,
                OrderPermission::ReplaceOnly,
                PositionAction::Hold,
                vec![AllowedGridMode::RangeGrid],
            )
        }
        MarketState::UptrendFollow => {
            reasons.push("上涨趋势确认，关闭普通网格，允许趋势跟随".into());
            (
                RiskLevel::SoftBlock,
                OrderPermission::ReplaceOnly,
                PositionAction::CloseGridOnly,
                vec![AllowedGridMode::UptrendFollow],
            )
        }
        MarketState::DownBreakWarning => {
            reasons.push("下跌破位预警，暂停新增买单".into());
            (
                RiskLevel::SoftBlock,
                OrderPermission::ReplaceOnly,
                PositionAction::Hold,
                vec![AllowedGridMode::RangeGrid],
            )
        }
        MarketState::DowntrendRisk => {
            reasons.push("下跌风险确认，禁止新开多头网格".into());
            (
                RiskLevel::HardBlock,
                OrderPermission::ReduceOnly,
                PositionAction::ReduceByRatio,
                vec![],
            )
        }
    }
}

/// 评估 RiskOverride 优先级。
/// 外部风控层可在分析后调用此函数覆盖。
pub fn evaluate_override(
    data_quality: &DataQuality,
    portfolio: Option<&PortfolioRiskInput>,
    rc: &RiskConfig,
) -> RiskOverride {
    // 数据质量过低
    if !data_quality.warmup_satisfied || data_quality.quality_score < 0.3 {
        return RiskOverride::DataQualityBlock;
    }

    // 账户级硬止损检查
    if let Some(pf) = portfolio {
        if let Some(dd) = pf.max_equity_drawdown {
            if dd > rc.max_drawdown * 1.5 {
                return RiskOverride::GlobalHardStop;
            }
        }
    }

    RiskOverride::None
}
