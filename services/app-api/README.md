# app-api

`app-api` 是业务 API 服务，负责读取数据库并对外提供查询接口。

当前第一版只提供行情 K 线查询。后面如果要查询 `users` 表、分析结果表、策略结果表，也建议继续放在这个 crate 里，而不是放到 `gateway`。

## 职责边界

```text
小程序 / 外部调用方
        |
        v
gateway：登录鉴权、注入用户身份、转发请求
        |
        v
app-api：读取 Postgres，提供业务接口
        |
        v
Postgres
        ^
        |
quote-ingester：拉取 Binance K 线并写入数据库
```

## 本地配置

从仓库根目录执行：

```bash
cp services/app-api/app-api.example.toml app-api.toml
```

设置数据库连接：

```bash
export DATABASE_URL='postgres://user:password@127.0.0.1:5433/mini_micro'
```

启动服务：

```bash
cargo run -p app-api
```

默认监听：

```text
127.0.0.1:9000
```

这个端口和 `services/gateway/gateway.example.toml` 里的 upstream 默认值一致：

```toml
[[upstreams]]
prefix = "/api/v1"
base_url = "http://127.0.0.1:9000"
```

所以小程序正常应该还是请求 gateway，由 gateway 转发到 app-api。

## 接口

### 健康检查

```http
GET /healthz
```

返回 `200 OK`。

### 查询 K 线

```http
GET /api/v1/market/klines
```

参数：

| 参数 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `source` | 否 | `binance` | 数据来源 |
| `symbol` | 是 | 无 | 交易对，例如 `BTCUSDT` |
| `interval` | 是 | 无 | 周期，例如 `1m`、`5m`、`30m` |
| `startTime` | 否 | 无 | 开始时间，毫秒时间戳，包含 |
| `endTime` | 否 | 无 | 结束时间，毫秒时间戳，不包含 |
| `limit` | 否 | `500` | 返回数量，最大 `1000` |

示例：

```bash
curl 'http://127.0.0.1:9000/api/v1/market/klines?symbol=BTCUSDT&interval=1m&limit=10'
```

响应示例：

```json
{
  "source": "binance",
  "symbol": "BTCUSDT",
  "interval": "1m",
  "items": [
    {
      "openTime": 1750493580000,
      "open": "102000.12",
      "high": "102100.45",
      "low": "101900.00",
      "close": "102050.30",
      "baseVolume": "12.345",
      "quoteVolume": "1260000.12",
      "sourceCount": 1,
      "isComplete": true
    }
  ]
}
```

价格和成交量用字符串返回，是为了避免 JavaScript `number` 的精度问题。

### 查询已有行情数据

```http
GET /api/v1/market/symbols
```

示例：

```bash
curl 'http://127.0.0.1:9000/api/v1/market/symbols'
```

响应示例：

```json
{
  "items": [
    {
      "source": "binance",
      "symbol": "BTCUSDT",
      "interval": "1m",
      "startTime": 1747890000000,
      "endTime": 1750493700000,
      "rowCount": 43200
    }
  ]
}
```

## 后续扩展建议

如果要增加 users 相关接口，建议新增：

```text
services/app-api/src/users.rs
```

并在 `app.rs` 里注册路由。

注意 users 接口不要一开始就做成公开查询。用户数据涉及隐私，应该先明确：

- 哪些接口必须登录。
- 当前用户只能查自己，还是管理员可以查全部。
- 是否信任 gateway 注入的 `x-user-id` 等身份 header。
