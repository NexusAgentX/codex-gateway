# codex-gateway 功能与实现说明

本文档描述仓库当前代码已经实现的功能、运行方式和边界。它取代项目启动时留下的设计草案，不再作为路线图或待办清单使用。

协议字段、SSE 事件和 Codex CLI 抓包证据见 [`docs/codex-protocol.md`](codex-protocol.md)；本地抓包实验环境见 [`docs/codex-mitm-test-env.md`](codex-mitm-test-env.md)。

## 1. 产品定位

codex-gateway 是一个面向 Codex Responses API 的单机聚合网关。它连接一个或多个已经可用的上游中转站，对下游提供统一的模型名、API Key、路由、限额、用量统计和管理面板。

网关不处理 OpenAI OAuth，不模拟官方登录流程，也不是通用 OpenAI API 网关。当前代理范围只覆盖 Codex CLI 已使用并经过验证的 Responses HTTP 接口。

## 2. 已实现的系统组成

```text
Codex CLI / API client
        |
        | cgk_live_* API key
        v
Rust + Axum gateway -------------- React management panel
        |                                      |
        | model mapping + routing              | cgw_panel_* token
        v                                      |
Codex-compatible upstreams <-------- SQLite storage
```

后端使用 Rust 2024、Axum、Tokio、Reqwest 和 SQLx；前端使用 React 19、TypeScript、Vite、Tailwind CSS、TanStack Query、React Router 和 Recharts。持久化层当前只支持 SQLite。

生产构建会把前端静态资源嵌入 Rust 二进制。浏览器路由使用 SPA fallback，未知的 `/api`、`/v1` 和 `/responses` 路径仍返回 `404`，不会误返回前端页面。

## 3. 用户、登录与 API Key

系统有 `admin` 和 `user` 两种角色，用户状态为 `active` 或 `disabled`。

### 3.1 面板登录

- 用户通过邮箱和密码调用 `POST /api/login`。
- 密码使用 Argon2 哈希存储。
- 登录成功后签发 `cgw_panel_*` HMAC 签名令牌，有效期 12 小时。
- 面板令牌只能访问用户 API 或管理 API，不能调用模型代理接口。
- 用户被禁用后，已经签发的面板令牌也会被拒绝。

### 3.2 下游 API Key

- Key 格式为 `cgk_live_{prefix}_{secret}`，使用系统随机源生成。
- 数据库只保存基于 `CODEX_GATEWAY_APP_SECRET` 的 HMAC 哈希和可检索 prefix，明文只在创建响应中返回一次。
- Key 支持名称、过期时间以及 `active`、`disabled`、`revoked` 状态。
- 普通用户只能查看、创建、禁用和撤销自己的 Key；管理员可以操作任意用户的 Key。
- 下游 API Key 可以访问所属用户范围内的面板 API，也可以调用代理接口。

### 3.3 管理员初始化

启动时可以通过以下变量创建或同步初始管理员：

```text
CODEX_GATEWAY_ADMIN_EMAIL
CODEX_GATEWAY_ADMIN_PASSWORD
CODEX_GATEWAY_BOOTSTRAP_ADMIN_KEY
```

Bootstrap Key 必须已经符合 `cgk_live_{prefix}_{secret}` 格式。重复启动会按配置协调同一管理员和同名 bootstrap key，不会不断创建新记录。

## 4. Codex 兼容接口

当前实现的模型接口如下：

| Method | Path | 行为 |
| --- | --- | --- |
| `POST` | `/responses` | Responses 请求代理，支持普通 JSON 和 SSE |
| `POST` | `/v1/responses` | 与 `/responses` 相同，转发时规范为 `/responses` |
| `POST` | `/responses/compact` | 远程上下文压缩请求代理 |
| `POST` | `/v1/responses/compact` | 与 `/responses/compact` 相同 |
| `GET` | `/v1/models` | 返回已启用且对用户可见的网关模型 |

代理接口只接受 `cgk_live_*` API Key。请求体必须是 JSON 并包含字符串 `model`；除模型名改写外，未知 JSON 字段保持不变。请求体大小由运行时 `max_request_body_bytes` 限制，默认 10 MiB。

当前明确不支持：

```text
POST /v1/chat/completions
POST /v1/images/generations
WebSocket /responses
POST /realtime/calls
POST /memories/trace_summarize
ANY  /codex/{path}
```

Codex CLI 应配置 `wire_api = "responses"`，且不要为此网关启用 WebSocket provider。

## 5. 代理行为

一次代理请求按以下顺序执行：

1. 校验下游 API Key、用户状态和过期时间。
2. 应用用户与 API Key 的请求额度、Token 预算、速率和并发限制。
3. 读取 JSON，提取 `model`、`stream` 和可安全处理的 `client_metadata`。
4. 查询已启用模型、映射和非 `down` 上游，并按当前策略排序。
5. 把公开模型名改写为上游模型名，替换 Authorization 后发往上游。
6. 透传普通响应或逐块转发 SSE，同时提取 usage。
7. 写入每次尝试的请求日志、每日聚合用量和限额计数。

### 5.1 Header 处理

向上游转发时会：

- 删除下游 `Authorization`、`Host`、`Content-Length`、Cookie、API Key 类敏感头和 hop-by-hop header。
- 使用上游 API Key 重新生成 `Authorization: Bearer ...`。
- 保留 `Accept`、`Content-Type`、`User-Agent`、`traceparent`、`tracestate`。
- 透传非敏感的 `x-codex-*`、`x-openai-*`、`x-responsesapi-*` 和 `openai-*` header。
- 添加 `x-codex-gateway: codex-gateway/0.1`。

返回下游时会过滤 Cookie、认证信息、`Server`、`X-Powered-By`、`Content-Length` 和 hop-by-hop header，其余上游响应头与状态码保持不变。

每个响应都包含 `x-request-id`。合法的客户端请求 ID 会被沿用，否则生成 UUID。发生 fallback 时，第一次尝试使用原 ID，后续日志使用 `{request-id}-2`、`{request-id}-3` 等后缀，最终下游响应仍使用原 ID。

启用调试响应头后，还会返回当前 `x-codex-gateway-route-strategy` 和 `x-codex-gateway-upstream-id`。该功能默认关闭。

### 5.2 SSE 与客户端断开

- SSE 数据从上游逐块转发，不缓存完整响应。
- 已经开始的流式请求不会切换上游或重试。
- 网关从 SSE 事件中增量提取 usage，并在 `response.completed`、EOF 或异常时完成日志。
- 客户端提前断开会取消上游流，释放并发占用，并以状态 `499`、错误码 `client_disconnected` 完成请求日志。

### 5.3 Usage

普通 JSON 响应和 SSE 最终事件中的 usage 都会被解析。实现兼容 Responses 常见的 `input_tokens`、`output_tokens`、`total_tokens`，并落为 `prompt_tokens`、`completion_tokens`、`total_tokens`。

有可信上游 usage 时 `usage_source` 为 `upstream`；没有 usage 时记录为 `unknown`。当前实现不根据字符数估算 Token。

## 6. 模型映射、路由与失败切换

管理员先创建公开模型，再为它添加一个或多个上游映射。映射包含上游、上游模型名、启用状态、优先级和权重。

只有同时满足以下条件的候选项会参与路由：

- 公开模型已启用。
- 模型映射已启用。
- 上游已启用。
- 上游健康状态不是 `down`；`healthy`、`degraded` 和初始 `unknown` 均可参与。

当前支持三种全局路由策略：

| 策略 | 行为 |
| --- | --- |
| `priority` | 先按映射优先级，再按上游优先级，选择最靠前的候选项 |
| `weighted` | 每个请求使用随机种子，按“映射权重 x 上游权重”确定候选顺序 |
| `sticky_by_key` | 优先使用 `client_metadata` 中的 session/thread/turn ID，否则使用 API Key ID，稳定地生成加权顺序 |

路由决定会以脱敏 JSON 写入请求日志，包含候选、优先级、权重和路由键哈希，不包含原始粘性值、上游密钥或上游 URL。

### 6.1 非流式重试

非流式请求在以下情况可以尝试下一个候选项：

- 连接失败。
- 请求或响应体读取超时。
- 上游返回 `502`、`503` 或 `504`。
- 上游 URL 或已存 Authorization 无法构造。

候选排序后的第一个上游的 `max_retries` 控制本次请求最多允许的后续尝试数。客户端错误和其他非临时状态不会重试；流式请求始终只尝试一次。

### 6.2 上游超时

每个上游可以使用自己的显式 `timeout_ms`，也可以跟随系统运行时默认值。跟随默认值的上游会立即使用面板中更新后的超时，无需重启。

## 7. 上游健康状态

上游健康状态为 `healthy`、`degraded`、`down` 或 `unknown`。系统同时记录最近检查时间、状态变化时间、最近降级/下线时间和一组脱敏错误样本。

健康信息来自两条路径：

- 后台任务按 `CODEX_GATEWAY_HEALTH_CHECK_INTERVAL_MS` 周期访问每个已启用上游的 `health_check_path`；该任务可以通过环境变量关闭。
- 每次真实代理请求也会根据连接错误、超时和 HTTP 状态更新健康状态。

管理员可以从面板或 `POST /api/admin/upstreams/{id}/health` 手动触发检查。健康检查使用上游自己的显式超时或当前运行时默认超时。

## 8. 限额与并发控制

系统可控制四类限制：

- 指定时间窗口内的请求额度。
- 指定时间窗口内的 Token 预算。
- 固定窗口请求速率。
- 同时进行中的请求数。

系统级策略是默认值；用户和 API Key 分别可以选择继承、设置具体限制或设为无限制。每次调用同时检查用户范围和 API Key 范围，任一范围不满足都会在访问上游前拒绝请求。

默认系统策略为全部无限制，请求与 Token 窗口默认 86400 秒，速率窗口默认 60 秒。管理员可以在 Settings、Users 和 API Keys 页面编辑对应策略；普通用户只能查看自己的有效策略、已用量、剩余量和重置时间。

额度不足返回 `403 quota_exceeded`，速率超限返回 `429 rate_limited`，并发超限返回 `429 concurrency_limited`。被拒绝的请求不会调用上游，也不会计入成功接纳后的 usage。

## 9. 请求日志、用量和分析

每次上游尝试都会写入 `request_logs`，主要包含：

- request、用户、API Key、模型和上游标识。
- 路径、状态码、错误码、是否流式和延迟。
- 输入/输出字节数和 Token usage。
- 上游 response ID/status、User-Agent。
- 脱敏后的 client metadata 字段名与少量标识哈希。
- 路由策略和脱敏后的路由决定。

系统不保存 prompt、completion、下游 Key、上游 Key、Cookie 或原始 client metadata 值。

`daily_usage` 按日期、用户、Key、模型和上游实时 upsert，记录请求数、错误数、流式请求数、Token 和延迟总和。

用户 API 始终强制限定为当前用户；管理员 API 支持全局查看。请求日志可以按用户（仅管理员）、Key、模型、上游、HTTP 状态或 `error`、时间范围、延迟范围过滤。用量可以按相同业务维度和日期范围过滤，列表 `limit` 会被限制在 1 到 1000。

分析接口和面板提供：

- 24 小时请求/错误趋势。
- 7 天 Token 用量。
- 模型请求占比。
- 上游和用户错误率。
- 延迟趋势与延迟分桶。
- 最近失败和可跳转到 Requests 页的钻取链接。

管理员 metrics 仅返回聚合计数、延迟、Token 和上游健康摘要，不暴露请求内容或凭据。

## 10. Web 管理面板

面板已实现响应式桌面/移动布局以及以下页面：

| 页面 | 普通用户 | 管理员 |
| --- | --- | --- |
| Overview | 自己的指标、限额、趋势、最近请求 | 全局指标、分析、异常上游、最近请求 |
| Usage | 自己的汇总、图表、模型和失败记录 | 全局维度、上游/用户钻取 |
| Requests | 查看并过滤自己的请求 | 查看并过滤全部请求 |
| API Keys | 创建、一次性复制、禁用、撤销、查看分 Key 用量 | 管理所有 Key，并配置分 Key 限额 |
| Upstreams | - | 创建、编辑、启停、密钥轮换、手动健康检查 |
| Models | - | 创建/编辑模型，管理上游映射、优先级和权重 |
| Users | - | 创建/编辑/禁用用户、改角色、重置密码、配置用户限额 |
| Settings | - | 查看/修改运行时配置、系统限额和资源计数 |

管理员专属路由同时由前端和后端角色校验，普通用户不能通过直接请求绕过。

## 11. 运行时配置

以下设置支持数据库保存并实时生效：

- `route_strategy`
- `default_request_timeout_ms`
- `max_request_body_bytes`
- `request_log_retention_days`
- `daily_usage_retention_days`
- `expose_debug_headers`

有效值优先级固定为：

```text
environment > database > built-in default
```

如果环境变量已设置，面板仍可保存数据库值，但当前运行值会继续被环境变量覆盖。监听地址、数据库 URL、密钥版本、CORS、日志级别、后台探活开关/周期和 bootstrap 配置只从环境读取，修改后需要重启。

## 12. 数据保留、审计与安全

### 12.1 数据保留

请求日志默认保留 90 天，每日用量默认保留 730 天。值为 `0` 时不删除该类数据。清理任务可以在启动时执行，也可以由管理员调用 `POST /api/admin/retention/run`；重复执行是幂等的。

### 12.2 管理操作审计

管理员执行的用户、Key、上游、模型、映射、限额和运行时设置变更，以及手动探活和数据清理，都会写入 `admin_audit_logs`。审计记录包含操作人、动作、资源、成功状态和脱敏元数据，不包含密码、API Key、Cookie、prompt 或 completion。

当前没有对外提供审计日志查询 API；记录保存在 SQLite 中供运维审计。

### 12.3 密钥保护

- 下游 Key 只保存 HMAC 哈希。
- 上游 Key 使用 ChaCha20-Poly1305 加密，记录格式为 `cgwenc_v1`，并保存密钥版本。
- 启动时会把旧版明文上游 Key 自动升级为当前加密格式。
- 提高 `CODEX_GATEWAY_SECRET_KEY_VERSION` 后，新建或重新保存的上游使用新版本加密。
- 非 development 环境要求 `CODEX_GATEWAY_APP_SECRET` 至少 32 个字符且不能使用默认值。

更换 `CODEX_GATEWAY_APP_SECRET` 会同时使现有面板令牌、下游 Key 哈希和上游密文失效，必须作为完整凭据轮换维护处理。

CORS 默认只允许 `CODEX_GATEWAY_PUBLIC_URL` 的 origin，并可通过 `CODEX_GATEWAY_PANEL_ORIGINS` 增加来源。

## 13. API 一览

### 13.1 公共与用户 API

```text
GET  /healthz
POST /api/login
GET  /api/me
GET  /api/models
GET  /api/overview
GET  /api/api-keys
POST /api/api-keys
GET  /api/api-keys/{id}/usage
POST /api/api-keys/{id}/disable
POST /api/api-keys/{id}/revoke
GET  /api/requests
GET  /api/analytics
GET  /api/usage/daily
GET  /api/usage/summary
GET  /api/limits
```

### 13.2 管理 API

```text
GET|POST /api/admin/users
PATCH    /api/admin/users/{id}
POST     /api/admin/users/{id}/password
GET|PATCH /api/admin/users/{id}/limits

GET|POST /api/admin/api-keys
GET      /api/admin/api-keys/{id}/usage
GET|PATCH /api/admin/api-keys/{id}/limits
POST     /api/admin/api-keys/{id}/disable
POST     /api/admin/api-keys/{id}/revoke

GET|POST /api/admin/upstreams
PATCH    /api/admin/upstreams/{id}
POST     /api/admin/upstreams/{id}/disable
POST     /api/admin/upstreams/{id}/health

GET|POST /api/admin/models
PATCH    /api/admin/models/{id}
GET|POST /api/admin/models/{id}/mappings
PATCH    /api/admin/model-mappings/{id}
POST     /api/admin/model-mappings/{id}/disable

GET       /api/admin/requests
GET       /api/admin/analytics
GET       /api/admin/usage/daily
GET       /api/admin/usage/summary
GET       /api/admin/metrics
GET       /api/admin/limits
PATCH     /api/admin/limits/system
GET|PATCH /api/admin/settings
POST      /api/admin/retention/run
```

网关自身生成的 API 错误使用统一结构；上游返回的错误响应仍按代理规则透传：

```json
{
  "error": {
    "message": "No healthy upstream available for model codex-mini",
    "type": "gateway_error",
    "code": "upstream_unavailable"
  }
}
```

限额错误还会在 `error.details` 中返回 scope、subject、限制值、已用量和重置时间。

## 14. 部署与数据边界

当前支持的部署形态是单个进程加单个 SQLite 数据库。服务启动时创建数据库父目录、执行 migrations、升级旧上游密文、同步 bootstrap 管理员、按配置清理过期数据并启动后台探活。

生产构建命令：

```bash
scripts/build-release.sh
```

生成的 `target/release/codex-gateway` 同时提供 API 和嵌入式面板。当前未实现 PostgreSQL、多实例共享限额状态、分布式路由或 Prometheus `/metrics`；不要按这些能力规划现有部署。
