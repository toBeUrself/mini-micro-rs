# 开发说明

本文说明如何在本地开发 `services/gateway`。

## 本地设置

安装 Rust stable toolchain 后，在仓库根目录运行：

```bash
cargo test --workspace
```

准备本地配置：

```bash
cp services/gateway/gateway.example.toml gateway.toml
```

设置本地环境变量：

```bash
export WECHAT_APP_SECRET='dev-secret'
export GATEWAY_JWT_SECRET='dev-jwt-secret'
export DATABASE_URL='postgres://user:password@127.0.0.1:5432/mini_micro_dev'
```

启动：

```bash
cargo run -p gateway
```

## 常用命令

| Command | Description |
| --- | --- |
| `cargo fmt` | 格式化 Rust 代码。 |
| `cargo fmt --check` | 检查格式是否符合 rustfmt。 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 对整个 workspace 运行 clippy，并把 warning 当成错误。 |
| `cargo test --workspace` | 运行全部测试。 |
| `cargo run -p gateway` | 启动网关服务。 |

## 代码风格

当前仓库使用 Rust 标准工具链：

- 格式化：`cargo fmt`。
- 静态检查：`cargo clippy --workspace --all-targets -- -D warnings`。
- 测试：Rust `#[test]` 和 `#[tokio::test]`。

新增代码应遵循现有模块边界：

- HTTP handler 放在 `app.rs`。
- 微信 API 调用放在 `wechat.rs`。
- 数据库存取放在 `store.rs`。
- 与认证转发相关的 header 逻辑放在 `proxy.rs`。
- 配置结构放在 `config.rs`。

## 注释约定

代码注释应解释边界、约束和不明显的业务规则，例如：

- 为什么需要清理 `x-user-*` 和 `x-wechat-*` 请求头。
- 为什么手机号绑定冲突不自动合并账号。
- 为什么微信 `access_token` 要提前 60 秒过期。

不需要给简单字段赋值、直观的 getter 或明显的函数调用加注释。

## 分支和 PR

仓库当前没有 `.github` PR 模板或分支命名规则。提交 PR 前至少应确认：

1. 变更范围和需求一致。
2. 新增行为有对应测试。
3. `cargo fmt --check` 通过。
4. `cargo clippy --workspace --all-targets -- -D warnings` 通过。
5. `cargo test --workspace` 通过。

## 新增功能建议

新增网关功能时优先保持以下设计：

- handler 只编排流程，不直接写 SQL。
- 外部服务调用封装成客户端，响应解析逻辑可单测。
- 存储层通过 trait 暴露能力，便于 handler 使用内存实现测试。
- 对下游注入的身份信息必须来自网关可信上下文，不能信任客户端传入的身份头。
