# 测试说明

当前测试集中在 `services/gateway/src` 的模块内单元测试和 handler 集成式测试。

## 测试框架

使用 Rust 内置测试框架：

- 同步单元测试：`#[test]`。
- 异步测试：`#[tokio::test]`。
- Axum 路由测试：`tower::ServiceExt::oneshot`。
- Mock HTTP 服务：测试内通过 `tokio::net::TcpListener` 启动本地 Axum server。

测试依赖定义在 `services/gateway/Cargo.toml` 的 `[dev-dependencies]`：

- `http-body-util`
- `tower`

## 运行测试

运行全部测试：

```bash
cargo test --workspace
```

运行 gateway 包测试：

```bash
cargo test -p gateway
```

运行单个测试：

```bash
cargo test -p gateway wechat_login_returns_jwt
```

显示测试输出：

```bash
cargo test -p gateway -- --nocapture
```

## 测试覆盖点

当前已有测试覆盖：

- TOML 配置解析和 upstream prefix 校验。
- 微信 `code2Session` 响应解析。
- 微信手机号响应解析和 `watermark.appid` 校验。
- JWT 签发、验证、过期和错误签名。
- 代理前清理客户端伪造身份头并注入网关身份头。
- `/healthz`。
- `wechat-login` 成功返回 JWT。
- `wechat-phone-login` 保存 `openid + phone`。
- `bind-phone` 缺少 JWT 返回 `401`。
- 受保护代理缺少 JWT 返回 `401`。
- 有效 JWT 请求转发到 mock upstream 并注入用户上下文头。
- 手机号账号冲突返回 `409 account_conflict`。

## 编写新测试

建议按模块放置测试：

- 配置解析测试放在 `config.rs`。
- 微信响应解析测试放在 `wechat.rs`。
- JWT 行为测试放在 `jwt.rs`。
- 代理 header 测试放在 `proxy.rs`。
- 端到端 handler 流程测试放在 `app.rs` 的测试模块。

新增 handler 测试优先使用内存版 `UserStore`，避免依赖真实 Postgres。需要测试微信行为时，使用本地 mock Axum server 替代真实微信 API。

## 覆盖率

仓库当前没有配置覆盖率阈值，也没有 CI workflow。合并前以以下命令作为最低验证标准：

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## 集成数据库测试

当前测试不连接真实 Postgres。需要增加数据库集成测试时，建议：

1. 使用独立测试数据库。
2. 测试启动前运行 migration。
3. 每个测试使用事务或唯一测试数据。
4. 显式覆盖唯一索引冲突和并发绑定场景。
