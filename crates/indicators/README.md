# indicators

纯 K 线技术指标计算库，零 IO 依赖，仅做数学运算。

## 设计原则

- **无副作用**：不访问网络、数据库、文件系统
- **纯函数**：输入数据数组，输出计算结果
- **NaN/Inf 安全**：非法值统一标记为 `Unavailable`，不静默替换为 0
- **warmup 透明**：数据不足时返回 `Unavailable` 并附带原因

## 指标列表

| 模块 | 指标 | 默认参数 |
|---|---|---|
| `ma` | MA20 / MA60 / EMA20，均线粘合，斜率，偏离率 | slope_lookback=5 |
| `boll` | BOLL（上/中/下轨），bandwidth，%B | period=20, mult=2.0 |
| `macd` | DIF / DEA / histogram，金叉/死叉 | 12/26/9 |
| `atr` | ATR（Wilder 平滑） | period=14 |
| `adx` | ADX / +DI / -DI / DX | period=14 |
| `rsi` | RSI（Wilder 平滑） | period=14 |
| `vol_ratio` | 成交量比率 | period=20 |
| `donchian` | Donchian Channel，突破判断 | period=20 |
| `price_structure` | swing high/low，HH/HL/LH/LL，区间边界 | pivot=2, lookback=20 |
| `percentile` | 滚动分位数，分位数分类 | window=1000 |
| `types` | 公共类型：IndicatorValue，SMA/EMA，滚动统计，线性评分函数 | — |

## 使用示例

```rust
use indicators::{boll, macd, atr, rsi, types::IndicatorValue};

let close = vec![100.0, 101.0, 102.0, ...];
let high = vec![...];
let low = vec![...];

// BOLL
let boll = boll::compute_boll(&close, 20, 2.0);
if let Some(upper) = boll.upper.last().and_then(|v| v.value()) {
    println!("BOLL upper: {upper}");
}

// MACD
let macd = macd::compute_macd(&close, 12, 26, 9);
if let Some(hist) = macd.hist.last().and_then(|v| v.value()) {
    println!("MACD hist: {hist}");
}

// ATR
let atr = atr::compute_atr(&high, &low, &close, 14);
if let Some(val) = atr.atr.last().and_then(|v| v.value()) {
    println!("ATR: {val}");
}

// RSI
let rsi_vals = rsi::compute_rsi(&close, 14);
if let Some(val) = rsi_vals.last().and_then(|v| v.value()) {
    println!("RSI: {val}");
}
```

## 数据类型

```rust
pub enum IndicatorValue {
    Available(f64),
    Unavailable(String),  // 附带不可用原因
}
```

所有指标返回 `Vec<IndicatorValue>`，与输入序列等长。调用方通过 `.value()` 获取 `Option<f64>`，通过 `.is_available()` 判断可用性。

## warmup 要求

| 指标 | 最小 K 线 | 建议 warmup |
|---|---|---|
| MA20 / EMA20 | 20 | 60 |
| MA60 | 60 | 120 |
| BOLL20 | 20 | 60 |
| MACD 12/26/9 | 35 | 120 |
| ATR14 | 15 | 100 |
| ADX14 | 28 | 150 |
| RSI14 | 15 | 100 |
| Volume Ratio20 | 20 | 60 |
| Donchian20 | 20 | 60 |

## 特性开关

- `serde`：为 `IndicatorValue` 等类型启用 serde 序列化支持（可选）

## 测试

```bash
cargo test -p indicators
```
