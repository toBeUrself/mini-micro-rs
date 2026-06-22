//! 网格计划生成：DisplayGridPlan + GridLevel + 交易所约束。

use rust_decimal::Decimal;
use crate::config::GridConfig;
use crate::models::{
    DisplayGridPlan, ExchangeConstraints, ExecutableGridPlan, GridLevel, GridMode, MarketState,
    OrderSide, RiskLevel,
};

/// 从 BOLL 指标和状态生成展示网格计划。
pub fn build_display_grid_plan(
    state: MarketState,
    boll_mid: Option<f64>,
    boll_upper: Option<f64>,
    boll_lower: Option<f64>,
    atr: Option<f64>,
    confidence: f64,
    gc: &GridConfig,
) -> DisplayGridPlan {
    let (mut enabled, mode) = match state {
        MarketState::RangeGrid => (true, GridMode::RangeGrid),
        MarketState::UpBreakWarning => (true, GridMode::RangeGrid),
        MarketState::UptrendFollow => (true, GridMode::UptrendFollow),
        MarketState::Wait | MarketState::DownBreakWarning | MarketState::DowntrendRisk => {
            (false, GridMode::Wait)
        }
    };

    let center = boll_mid;
    let upper = boll_upper;
    let lower = boll_lower;

    let (boundary_mode, planned_lower, planned_upper) = match gc.boundary_mode.as_str() {
        "boll" => (gc.boundary_mode.clone(), lower, upper),
        _ => ("boll".into(), lower, upper),
    };

    let grid_step = match (planned_lower, planned_upper) {
        (Some(l), Some(u)) if enabled && gc.grid_count > 0 && u > l => {
            let width = u - l;
            Some(width / gc.grid_count as f64)
        }
        _ => None,
    };

    if let (Some(c), Some(l), Some(u)) = (center, planned_lower, planned_upper) {
        let width = u - l;
        if c <= 0.0 || width <= 0.0 {
            enabled = false;
        } else {
            let width_pct = width / c;
            if width_pct > gc.max_grid_width_by_percent {
                enabled = false;
            }
            if let Some(atr_val) = atr {
                if atr_val.is_finite() && atr_val > 0.0 && width / atr_val > gc.max_grid_width_by_atr {
                    enabled = false;
                }
            }
        }
    } else {
        enabled = false;
    }

    DisplayGridPlan {
        enabled,
        mode: if enabled { mode } else { GridMode::Wait },
        boundary_mode,
        lower: planned_lower,
        upper: planned_upper,
        center,
        grid_count: if enabled { gc.grid_count } else { 0 },
        grid_step: if enabled { grid_step } else { None },
        risk_level: if enabled { RiskLevel::Advisory } else { RiskLevel::SoftBlock },
        confidence,
    }
}

/// 生成可执行网格计划（含 GridLevel）。
///
/// `capital_per_buy_level` 是每个买入网格层的预算。没有该信息时不能构造可执行订单计划。
pub fn build_executable_grid_plan(
    display: &DisplayGridPlan,
    constraints: Option<&ExchangeConstraints>,
    capital_per_buy_level: Option<Decimal>,
) -> ExecutableGridPlan {
    let Some(lower) = display.lower else { return disabled_plan(display.mode); };
    let Some(upper) = display.upper else { return disabled_plan(display.mode); };
    let Some(step) = display.grid_step else { return disabled_plan(display.mode); };
    if !display.enabled || display.grid_count == 0 || upper <= lower || step <= 0.0 {
        return disabled_plan(display.mode);
    }

    let max_levels = constraints.and_then(|c| c.max_open_orders).unwrap_or(usize::MAX);
    let mut levels = Vec::new();
    let mut total_capital = Decimal::ZERO;
    let mut executable_count = 0usize;
    let level_budget = capital_per_buy_level.unwrap_or(Decimal::ZERO);

    for i in 0..display.grid_count {
        let buy_price = lower + step * i as f64;
        let sell_price = buy_price + step;

        let raw_buy_price = decimal_from_f64(buy_price);
        let raw_sell_price = decimal_from_f64(sell_price);
        let raw_qty = if level_budget > Decimal::ZERO && raw_buy_price > Decimal::ZERO {
            level_budget / raw_buy_price
        } else {
            Decimal::ZERO
        };

        let mut buy_level = make_grid_level(OrderSide::Buy, raw_buy_price, raw_qty, constraints);
        let mut sell_level = make_grid_level(OrderSide::Sell, raw_sell_price, raw_qty, constraints);

        if levels.len() >= max_levels {
            mark_disabled(&mut buy_level, "max_open_orders exceeded");
        }
        levels.push(buy_level);

        if levels.len() >= max_levels {
            mark_disabled(&mut sell_level, "max_open_orders exceeded");
        }
        levels.push(sell_level);
    }

    for level in &levels {
        if level.executable {
            executable_count += 1;
        }
        if level.side == OrderSide::Buy && level.executable {
            total_capital += level.notional;
        }
    }

    ExecutableGridPlan {
        enabled: display.enabled && executable_count > 0,
        mode: display.mode,
        levels,
        total_required_capital: total_capital,
        executable_level_count: executable_count,
    }
}

fn disabled_plan(mode: GridMode) -> ExecutableGridPlan {
    ExecutableGridPlan {
        enabled: false,
        mode,
        levels: vec![],
        total_required_capital: Decimal::ZERO,
        executable_level_count: 0,
    }
}

fn decimal_from_f64(v: f64) -> Decimal {
    Decimal::try_from(v).unwrap_or(Decimal::ZERO)
}

fn make_grid_level(
    side: OrderSide,
    raw_price: Decimal,
    raw_qty: Decimal,
    constraints: Option<&ExchangeConstraints>,
) -> GridLevel {
    let mut executable = raw_price > Decimal::ZERO && raw_qty > Decimal::ZERO;
    let mut disabled_reason: Option<String> = if executable { None } else { Some("raw price or qty is zero".into()) };

    let (price, qty) = if let Some(c) = constraints {
        let rounded_price = floor_to_step(raw_price, c.tick_size).round_dp(c.price_precision);
        let rounded_qty = floor_to_step(raw_qty, c.step_size).round_dp(c.quantity_precision);
        let notional = rounded_price * rounded_qty;

        if rounded_qty < c.min_qty {
            executable = false;
            disabled_reason = Some(format!("qty below min_qty: {rounded_qty}"));
        }
        if notional < c.min_notional {
            executable = false;
            disabled_reason = Some(format!("notional below min_notional: {notional}"));
        }
        (rounded_price, rounded_qty)
    } else {
        (raw_price, raw_qty)
    };

    let notional = price * qty;

    GridLevel {
        side,
        raw_price,
        price,
        raw_qty,
        qty,
        notional,
        executable,
        disabled_reason,
    }
}

fn floor_to_step(value: Decimal, step: Decimal) -> Decimal {
    if value <= Decimal::ZERO || step <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    (value / step).trunc() * step
}

fn mark_disabled(level: &mut GridLevel, reason: &str) {
    level.executable = false;
    level.disabled_reason = Some(reason.into());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_floor_to_step() {
        assert_eq!(floor_to_step(Decimal::new(12345, 2), Decimal::new(1, 1)), Decimal::new(1234, 1));
    }
}
