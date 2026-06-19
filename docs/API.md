# API 说明

本文描述 `services/gateway` 当前暴露的 HTTP 接口。所有路径均来自 `services/gateway/src/app.rs` 和 `services/gateway/src/proxy.rs`。

## 认证机制

认证接口返回业务 JWT。访问受保护业务路由时，客户端必须带上：

```http
Authorization: Bearer <access_token>
```

JWT 使用 HS256 签名，密钥来自配置项 `jwt.secret_env` 指向的环境变量。Token 的 `sub` 是用户表中的 `users.id`。

## 响应格式

认证成功响应：

```json
{
  "token_type": "Bearer",
  "access_token": "<jwt>",
  "expires_in": 604800,
  "user": {
    "id": "00000000-0000-0000-0000-000000000000",
    "openid_bound": true,
    "phone_verified": false,
    "country_code": null,
    "phone_number": null
  }
}
```

错误响应：

```json
{
  "error": "unauthorized",
  "message": "missing or invalid bearer token"
}
```

## 端点列表

| Method | Path | Auth | Description |
| --- | --- | --- | --- |
| `GET` | `/healthz` | 否 | 健康检查，成功返回 `200 OK`。 |
| `POST` | `/api/v1/auth/wechat-login` | 否 | 使用微信小程序登录 code 登录或创建用户，并签发 JWT。 |
| `POST` | `/api/v1/auth/wechat-phone-login` | 否 | 使用登录 code 和手机号授权 code 一次完成微信身份绑定、手机号绑定和 JWT 签发。 |
| `POST` | `/api/v1/auth/bind-phone` | 是 | 当前登录用户绑定或更新手机号。 |
| fallback | 配置 upstream prefix 命中的其他路径 | 是 | 验证 JWT 后转发到配置的 upstream。 |

## `POST /api/v1/auth/wechat-login`

请求：

```json
{
  "code": "wx-login-code"
}
```

处理流程：

1. 网关校验 `code` 非空。
2. 调用微信 `/sns/jscode2session` 获取 `openid`、`unionid` 和 `session_key`。
3. 使用 `openid` 查找或创建用户。
4. 返回业务 JWT 和用户摘要。

说明：`session_key` 只在后端解析过程中使用，当前版本不会下发给前端，也不会持久化。

## `POST /api/v1/auth/wechat-phone-login`

请求：

```json
{
  "login_code": "wx-login-code",
  "phone_code": "wx-phone-code"
}
```

处理流程：

1. `login_code` 通过微信 `code2Session` 换取 `openid/unionid`。
2. `phone_code` 通过微信 `/wxa/business/getuserphonenumber` 换取手机号。
3. 校验手机号响应里的 `watermark.appid` 必须等于配置里的 `wechat.app_id`。
4. 在同一个用户身份上绑定微信身份和手机号。
5. 如果 `openid` 和手机号已经分别属于不同用户，返回 `409 account_conflict`。

## `POST /api/v1/auth/bind-phone`

请求头：

```http
Authorization: Bearer <access_token>
```

请求体：

```json
{
  "code": "wx-phone-code"
}
```

处理流程：

1. 验证 JWT 并加载当前用户。
2. 使用手机号授权 `code` 调用微信手机号接口。
3. 校验 `watermark.appid`。
4. 将手机号绑定到当前用户。
5. 如果手机号已经属于其他用户，返回 `409 account_conflict`。

## 代理转发

除显式注册的认证和健康检查路由外，其他请求进入 fallback 代理。代理只会匹配 `gateway.toml` 里的 `[[upstreams]]`：

```toml
[[upstreams]]
prefix = "/api/v1"
base_url = "http://127.0.0.1:9000"
```

匹配规则：

- 使用最长 prefix 匹配。
- prefix 必须按路径边界匹配，例如 `/api/v1` 匹配 `/api/v1/orders`，不匹配 `/api/v10/orders`。
- 转发时保留原始 path 和 query。

转发前会移除以下客户端请求头：

- `Authorization`
- `Host`
- `Content-Length`
- 所有 hop-by-hop headers
- `x-gateway-authenticated`
- 所有 `x-user-*`
- 所有 `x-wechat-*`

然后注入：

```http
x-gateway-authenticated: true
x-user-id: <uuid>
x-openid-bound: true|false
x-phone-verified: true|false
```

当前版本默认不向下游注入完整手机号，避免 PII 扩散。

## 常见状态码

| Status | Error | Meaning |
| --- | --- | --- |
| `400` | `bad_request` | 请求字段为空，或微信 API 返回业务错误。 |
| `401` | `unauthorized` | 缺少 JWT、JWT 无效、JWT 过期，或用户不存在。 |
| `404` | `not_found` | 没有匹配的 upstream prefix。 |
| `409` | `account_conflict` | `openid` 和手机号命中了不同用户。 |
| `502` | `bad_gateway` | 微信或 upstream HTTP 调用失败。 |
| `500` | `internal_error` | 数据库或其他内部错误。 |
