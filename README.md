# mini-micro-rs

`mini-micro-rs` 是一个 Rust/Axum API 网关，当前实现微信小程序登录、手机号绑定、业务 JWT 签发，以及向下游 HTTP 服务转发已认证请求。

## 功能概览

- 微信小程序静默登录：前端提交 `wx.login()` 返回的 `code`，网关调用微信 `code2Session` 换取 `openid/unionid`。
- 微信手机号绑定：前端提交手机号授权 `code`，网关调用微信手机号接口并校验 `watermark.appid`。
- 组合登录：首次授权手机号时可一次提交 `login_code + phone_code`，创建或查找用户并绑定手机号。
- JWT 认证：业务接口使用 `Authorization: Bearer <token>` 访问。
- 受保护代理：网关清理客户端伪造的身份头，再注入可信用户上下文头转发到配置的 upstream。
- Postgres 存储：用户表通过 sqlx migration 创建，`openid` 和手机号都有部分唯一索引。

## 项目结构

```text
.
├── Cargo.toml
├── services/gateway
│   ├── Cargo.toml
│   ├── gateway.example.toml
│   ├── migrations/
│   └── src/
└── docs/
```

主要代码在 `services/gateway/src/`：

- `app.rs`：Axum 路由、认证接口、鉴权入口。
- `wechat.rs`：微信接口客户端和响应解析。
- `store.rs`：用户存储接口与 Postgres 实现。
- `jwt.rs`：JWT 签发和验证。
- `proxy.rs`：受保护 HTTP 转发与身份头清洗。
- `config.rs`：TOML 配置解析和环境变量解析。

## 快速开始

前置条件：

- Rust stable toolchain
- 可访问的 Postgres 数据库
- 微信小程序 `app_id` 和 `app_secret`

安装依赖和验证构建：

```bash
cargo test --workspace
```

创建本地配置：

```bash
cp services/gateway/gateway.example.toml gateway.toml
```

设置运行时环境变量：

```bash
export WECHAT_APP_SECRET='your-wechat-app-secret'
export GATEWAY_JWT_SECRET='change-me-to-a-long-random-secret'
export DATABASE_URL='postgres://user:password@127.0.0.1:5432/mini_micro'
```

启动网关：

```bash
cargo run -p gateway
```

如果配置文件不在仓库根目录，使用 `GATEWAY_CONFIG` 指定路径：

```bash
GATEWAY_CONFIG=services/gateway/gateway.example.toml cargo run -p gateway
```

## 常用命令

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## 文档

- [架构说明](docs/ARCHITECTURE.md)
- [API 说明](docs/API.md)
- [配置说明](docs/CONFIGURATION.md)
- [开发说明](docs/DEVELOPMENT.md)
- [测试说明](docs/TESTING.md)
- [部署说明](docs/DEPLOYMENT.md)

## License

Workspace metadata declares the project license as `MIT` in `Cargo.toml`.
