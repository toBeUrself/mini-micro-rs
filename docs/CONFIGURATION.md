# 配置说明

网关使用 TOML 文件描述非敏感配置，使用环境变量提供敏感值和部署环境相关值。

默认配置文件路径是仓库运行目录下的 `gateway.toml`。也可以通过 `GATEWAY_CONFIG` 指定：

```bash
GATEWAY_CONFIG=/path/to/gateway.toml cargo run -p gateway
```

示例文件位于 `services/gateway/gateway.example.toml`。

## TOML 格式

最小配置示例：

```toml
[server]
bind = "127.0.0.1:8080"

[wechat]
app_id = "wx-your-mini-program-app-id"
app_secret_env = "WECHAT_APP_SECRET"

[jwt]
secret_env = "GATEWAY_JWT_SECRET"
ttl_seconds = 604800

[database]
url_env = "DATABASE_URL"

[[upstreams]]
prefix = "/api/v1"
base_url = "http://127.0.0.1:9000"
```

## 配置项

| Key | Required | Default | Description |
| --- | --- | --- | --- |
| `server.bind` | 是 | 无 | 网关监听地址，例如 `127.0.0.1:8080`。 |
| `wechat.app_id` | 是 | 无 | 微信小程序 AppID。 |
| `wechat.app_secret_env` | 是 | 无 | 保存微信 AppSecret 的环境变量名。 |
| `wechat.api_base` | 否 | `https://api.weixin.qq.com` | 微信 API base URL，测试时可指向 mock server。 |
| `jwt.secret_env` | 是 | 无 | 保存 JWT HS256 密钥的环境变量名。 |
| `jwt.ttl_seconds` | 否 | `604800` | JWT 有效期，单位秒。 |
| `database.url_env` | 是 | 无 | 保存 Postgres DSN 的环境变量名。 |
| `upstreams[].prefix` | 否 | 无 | 需要代理的路径前缀，必须以 `/` 开头。 |
| `upstreams[].base_url` | 否 | 无 | 下游服务地址，必须以 `http://` 或 `https://` 开头。 |

## 环境变量

`gateway.example.toml` 默认引用以下变量：

| Variable | Required | Description |
| --- | --- | --- |
| `WECHAT_APP_SECRET` | 是 | 微信小程序 AppSecret。 |
| `GATEWAY_JWT_SECRET` | 是 | JWT 签名密钥。生产环境应使用足够长的随机字符串。 |
| `DATABASE_URL` | 是 | Postgres 连接串，例如 `postgres://user:password@host:5432/dbname`。 |
| `GATEWAY_CONFIG` | 否 | 配置文件路径。不设置时读取 `gateway.toml`。 |
| `RUST_LOG` | 否 | tracing 日志过滤规则，例如 `info` 或 `gateway=debug,tower_http=info`。 |

注意：`wechat.app_secret_env`、`jwt.secret_env`、`database.url_env` 的值是环境变量名，不是密钥本身。

## 启动时校验

启动过程中会执行以下校验：

- 配置文件必须能被读取并解析为 TOML。
- `upstreams[].prefix` 必须以 `/` 开头。
- `upstreams[].base_url` 必须以 `http://` 或 `https://` 开头。
- `wechat.app_secret_env`、`jwt.secret_env`、`database.url_env` 指向的环境变量必须存在且非空。
- Postgres 必须可连接。
- `services/gateway/migrations` 下的 sqlx migration 必须能成功执行。

## 数据库迁移

当前 migration 创建：

- `users` 表。
- `users_openid_unique` 部分唯一索引：`openid IS NOT NULL` 时唯一。
- `users_phone_unique` 部分唯一索引：`country_code` 和 `pure_phone_number` 都非空时唯一。

应用启动时会自动运行：

```rust
sqlx::migrate!("./migrations").run(store.pool()).await?;
```

## 多环境配置

仓库当前没有内置 `.env` 加载逻辑。推荐每个环境提供独立 TOML 文件和独立环境变量：

```bash
GATEWAY_CONFIG=/etc/mini-micro-rs/gateway.production.toml
WECHAT_APP_SECRET=...
GATEWAY_JWT_SECRET=...
DATABASE_URL=...
```

部署系统应负责注入这些变量，避免把真实密钥提交到仓库。
