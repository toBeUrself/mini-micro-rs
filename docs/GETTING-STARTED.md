# 快速上手

本文面向第一次运行 `mini-micro-rs` 网关的人。

## 前置条件

- Rust stable toolchain。
- Postgres 数据库。
- 微信小程序 `app_id` 和 `app_secret`。
- 一个可接收代理请求的下游 HTTP 服务。

## 安装步骤

克隆仓库后进入项目目录：

```bash
git clone <repo-url>
cd mini-micro-rs
```

确认 Rust workspace 能编译：

```bash
cargo test --workspace
```

## 准备配置

复制示例配置到默认路径：

```bash
cp services/gateway/gateway.example.toml gateway.toml
```

编辑 `gateway.toml`：

- `server.bind`：本地监听地址。
- `wechat.app_id`：微信小程序 AppID。
- `upstreams[].prefix`：需要代理的路径前缀。
- `upstreams[].base_url`：下游服务地址。

设置环境变量：

```bash
export WECHAT_APP_SECRET='your-wechat-app-secret'
export GATEWAY_JWT_SECRET='change-me-to-a-long-random-secret'
export DATABASE_URL='postgres://user:password@127.0.0.1:5432/mini_micro'
```

## 第一次运行

启动网关：

```bash
cargo run -p gateway
```

启动时会连接 Postgres 并自动执行 `services/gateway/migrations` 下的 migration。

健康检查：

```bash
curl -i http://127.0.0.1:8080/healthz
```

预期返回 `200 OK`。

## 常见问题

### 启动时报环境变量缺失

检查 `gateway.toml` 中的 `wechat.app_secret_env`、`jwt.secret_env`、`database.url_env`。这些字段的值是环境变量名，实际密钥必须通过对应环境变量提供。

### 找不到配置文件

默认读取当前工作目录下的 `gateway.toml`。如果从其他目录启动，显式指定：

```bash
GATEWAY_CONFIG=/absolute/path/to/gateway.toml cargo run -p gateway
```

### Postgres 连接失败

确认 `DATABASE_URL` 指向的数据库存在，用户有建表和建索引权限。应用启动时会创建 `pgcrypto` extension 和 `users` 表。

### 代理请求返回 404

确认请求路径命中了 `[[upstreams]]` 的 `prefix`，且 prefix 按路径边界匹配。例如 `/api/v1` 会匹配 `/api/v1/orders`，不会匹配 `/api/v10/orders`。

## 下一步

- 阅读 [API 说明](API.md) 对接前端和下游服务。
- 阅读 [配置说明](CONFIGURATION.md) 完成生产配置。
- 阅读 [开发说明](DEVELOPMENT.md) 了解代码结构和本地开发流程。
