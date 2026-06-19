# 部署说明

当前仓库没有 Dockerfile、docker-compose、Kubernetes manifest 或 CI/CD workflow。部署方式应由外层平台提供，本服务本身只需要一个可运行的 Rust binary、Postgres、微信小程序配置和下游 upstream 配置。

## 部署目标

已在仓库中实现的运行方式：

- 编译并运行 `gateway` binary。
- 通过 TOML 文件配置监听地址、微信 AppID、JWT TTL 和 upstream。
- 通过环境变量注入微信 AppSecret、JWT secret 和 Postgres DSN。

TLS、HTTPS 合法域名、证书、WAF、限流和公网入口不在当前 Rust 服务内实现，应由 Nginx、Caddy、Ingress、云负载均衡或其他外层网关负责。

## 构建

在目标平台或构建机上运行：

```bash
cargo build --release -p gateway
```

产物路径：

```text
target/release/gateway
```

## 运行

准备配置文件，例如 `/etc/mini-micro-rs/gateway.toml`：

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

设置环境变量并启动：

```bash
export GATEWAY_CONFIG=/etc/mini-micro-rs/gateway.toml
export WECHAT_APP_SECRET='...'
export GATEWAY_JWT_SECRET='...'
export DATABASE_URL='postgres://user:password@host:5432/dbname'
export RUST_LOG='info'

./target/release/gateway
```

## 数据库

服务启动时会自动执行 sqlx migration：

```text
services/gateway/migrations/0001_create_users.sql
```

数据库用户需要有以下能力：

- 连接目标数据库。
- 创建 `pgcrypto` extension。
- 创建表。
- 创建索引。

如果生产数据库不允许应用自动创建 extension 或 schema，应由 DBA 或迁移流程提前执行 migration。

## 上线前检查

上线前至少检查：

1. `server.bind` 是否只监听预期网卡。若外层反向代理同机部署，通常监听 `127.0.0.1:<port>`。
2. `WECHAT_APP_SECRET` 是否来自密钥系统，而不是写在 TOML 文件中。
3. `GATEWAY_JWT_SECRET` 是否足够长且只在服务端保存。
4. `DATABASE_URL` 是否使用最小权限账号。
5. `upstreams[].base_url` 是否指向可信内网服务。
6. 外层 HTTPS 域名是否已配置到微信小程序合法域名。
7. 下游服务是否只信任网关注入的 `x-gateway-authenticated` 和 `x-user-id`。

## 回滚

仓库当前没有内置发布系统。通用回滚方式：

1. 保留上一版 binary 和对应配置。
2. 如果只改代码，停止当前进程并启动上一版 binary。
3. 如果包含数据库 migration，先确认 migration 是否向后兼容；当前 `0001_create_users.sql` 是创建初始表结构，没有 down migration。
4. 回滚后调用 `/healthz` 并执行一条登录或代理链路验证。

## 监控

当前代码使用 `tracing` 输出日志，未集成 Sentry、Datadog、New Relic 或 OpenTelemetry exporter。

建议生产环境至少采集：

- 进程存活和重启次数。
- `/healthz`。
- HTTP 5xx 比例。
- 微信 API 调用失败率。
- Postgres 连接错误。
- `409 account_conflict` 数量。
- upstream 502 数量和延迟。
