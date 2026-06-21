# quote-ingester

`quote-ingester` 是一个常驻运行的 K 线采集服务。它从 Binance `uiKlines` 接口拉取 `1m` K 线数据，保存到 Postgres，并在本地自动聚合生成 `5m`、`30m` K 线，供后续分析服务读取。

## 功能概览

- 从 `https://www.binance.com/api/v3/uiKlines` 拉取行情数据。
- 默认采集 `BTCUSDT` 的 `1m` K 线。
- 启动时回填最近 30 天数据。
- 持续轮询最新 K 线，默认每 20 秒同步一次。
- 本地从 `1m` 聚合生成 `5m` 和 `30m`。
- 使用 upsert 写入，重复拉到同一根 K 线时会覆盖更新。
- 用 `source_count` 和 `is_complete` 标记聚合 K 线是否完整。

## 项目结构

```text
services/quote-ingester
├── Cargo.toml
├── quote-ingester.example.toml
└── src/
    ├── aggregate.rs
    ├── api.rs
    ├── config.rs
    ├── main.rs
    ├── models.rs
    ├── store.rs
    └── worker.rs
```

主要模块：

- `api.rs`：调用 Binance K 线接口，并把数组格式响应转成内部模型。
- `aggregate.rs`：把 `1m` K 线聚合成 `5m`、`30m`。
- `config.rs`：读取 TOML 配置和环境变量。
- `store.rs`：负责 Postgres 查询和 upsert。
- `worker.rs`：常驻任务，负责回填、轮询和触发聚合。
- `main.rs`：进程入口，初始化日志、读取配置、运行 migration、启动 worker。

数据库 migration 放在仓库根目录 `migrations/`。这是因为 gateway 和 quote-ingester 使用同一个 Postgres 数据库，SQLx 的 `_sqlx_migrations` 表也是数据库级别共享的；每个会自动运行 migration 的服务都必须看到同一套 migration 历史。

## 快速开始

前置条件：

- Rust stable toolchain
- 本地 Postgres 已启动
- 已设置 `DATABASE_URL`

当前仓库的 Docker Postgres 默认连接地址是：

```bash
export DATABASE_URL='postgres://user:password@127.0.0.1:5433/mini_micro'
```

创建本地配置：

```bash
cp services/quote-ingester/quote-ingester.example.toml quote-ingester.toml
```

启动服务：

```bash
cargo run -p quote-ingester
```

如果配置文件不在仓库根目录，使用 `QUOTE_INGESTER_CONFIG` 指定：

```bash
QUOTE_INGESTER_CONFIG=services/quote-ingester/quote-ingester.example.toml cargo run -p quote-ingester
```

## 配置说明

示例配置：

```toml
[database]
url_env = "DATABASE_URL"

[quote_api]
source = "binance"
base_url = "https://www.binance.com"
timeout_seconds = 10

[[markets]]
symbol = "BTCUSDT"
source_interval = "1m"
limit = 1000
poll_seconds = 20
backfill_days = 30
derived_intervals = ["5m", "30m"]
```

字段含义：

- `database.url_env`：保存数据库连接字符串的环境变量名。
- `quote_api.source`：数据来源标识，默认是 `binance`。
- `quote_api.base_url`：行情接口域名。
- `quote_api.timeout_seconds`：单次 HTTP 请求超时时间。
- `markets.symbol`：交易对，例如 `BTCUSDT`。
- `markets.source_interval`：远端拉取周期，第一版默认使用 `1m`。
- `markets.limit`：每次接口请求最多拉取多少根 K 线。
- `markets.poll_seconds`：实时轮询间隔。
- `markets.backfill_days`：启动时回填最近多少天。
- `markets.derived_intervals`：从 `source_interval` 本地聚合出的周期。

## 数据表

服务会创建 `klines` 表。所有交易对和周期共用一张表，通过下面几个字段区分：

```sql
source, symbol, interval, open_time
```

唯一键：

```sql
(source, symbol, interval, open_time)
```

所以这些数据可以同时存在，不会互相覆盖：

```text
binance / BTCUSDT / 1m  / 2026-06-21 10:00:00
binance / BTCUSDT / 5m  / 2026-06-21 10:00:00
binance / BTCUSDT / 30m / 2026-06-21 10:00:00
```

核心字段：

- `open_price`：开盘价
- `high_price`：最高价
- `low_price`：最低价
- `close_price`：收盘价
- `base_volume`：基础币成交量，对应 Binance 响应里的 `volume`
- `quote_volume`：计价币成交额，对应 Binance 响应里的 `quoteAssetVolume`
- `source_count`：聚合时参与计算的源 K 线数量
- `is_complete`：聚合窗口是否完整

## 聚合规则

`5m` 和 `30m` 不直接从远端拉取，而是由本地 `1m` 数据聚合生成。

聚合规则：

- `open_price`：窗口内第一根 `1m` 的开盘价
- `close_price`：窗口内最后一根 `1m` 的收盘价
- `high_price`：窗口内最高价
- `low_price`：窗口内最低价
- `base_volume`：窗口内 `base_volume` 求和
- `quote_volume`：窗口内 `quote_volume` 求和

完整性规则：

- `5m` 需要 5 根 `1m`
- `30m` 需要 30 根 `1m`
- 如果数量不足，仍会生成聚合 K 线，但 `is_complete = false`

## 查看数据

在 DBeaver 或 psql 里可以用下面的 SQL 查看数据概况：

```sql
SELECT source, symbol, interval, count(*) AS rows,
       min(open_time) AS first_open_time,
       max(open_time) AS last_open_time
FROM klines
GROUP BY source, symbol, interval
ORDER BY source, symbol, interval;
```

查看最近 10 根 `1m`：

```sql
SELECT *
FROM klines
WHERE source = 'binance'
  AND symbol = 'BTCUSDT'
  AND interval = '1m'
ORDER BY open_time DESC
LIMIT 10;
```

查看不完整的聚合 K 线：

```sql
SELECT *
FROM klines
WHERE interval IN ('5m', '30m')
  AND is_complete = false
ORDER BY open_time DESC;
```

## 常用命令

运行测试：

```bash
cargo test -p quote-ingester
```

运行整个 workspace 测试：

```bash
cargo test --workspace
```

格式化代码：

```bash
cargo fmt
```

## 注意事项

- 第一版只从远端拉 `1m`，`5m/30m` 是本地聚合结果。
- 未收盘 K 线会被后续轮询覆盖更新，这是预期行为。
- 如果服务停机导致某些 `1m` 缺失，对应聚合 K 线会标记为 `is_complete = false`。
- 之前写入的 `azverse` 数据不会被自动删除；切换到 Binance 后，新数据会以 `source = 'binance'` 保存。
- 如果后续需要和交易所官方 `5m/30m` 完全对齐，可以再增加直接拉取官方周期或校验修正逻辑。
