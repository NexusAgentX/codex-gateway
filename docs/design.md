# codex-gateway 设计稿

## 1. 项目定位

codex-gateway 是一个面向 Codex 中转站的 LLM 聚合网关。

它不做 OpenAI OAuth 逆向，也不直接模拟官方登录流程；它只对接已经可用的 Codex 中转站上游，并向下游用户提供统一、稳定、可管理的访问入口。

核心目标：

- 下游用户使用 codex-gateway 时，体验尽量等同于直接使用某个上游中转站。
- 管理员可以配置多个上游中转站，并控制路由、限流、额度、可用模型和优先级。
- 普通用户可以自助创建 API Key，查看自己的调用记录、用量和错误情况。
- 系统保存完整使用记录，支持基础统计分析、审计和故障排查。
- 初版保持简单，但核心代理能力、流式转发、鉴权、用量记录和后台配置必须可靠。

## 1.1 文档分工

- `docs/design.md` 是产品与工程设计稿：看功能范围、架构、数据库、路由、重试、统计、Web 面板和里程碑。
- `docs/codex-protocol.md` 是协议事实与抓包证据文档：看 Codex CLI 与上游中转站之间的 endpoint、header、请求/响应字段、SSE 事件、usage/error 结构和 MITM 样本摘要。

如果文档之间出现冲突：

- 协议形态、路径和字段以 `docs/codex-protocol.md` 为准。
- 产品能力、落库字段、路由/重试策略和 UI 行为以本文档为准。

## 2. 角色与权限

### 管理员

管理员负责系统配置和运营。

能力：

- 登录 Web 面板。
- 管理用户：创建、禁用、改角色、重置额度。
- 管理上游：新增、编辑、禁用、健康检查、权重与优先级。
- 管理模型：配置模型映射、别名、可见性、默认路由。
- 查看全局调用记录、错误日志、用量统计。
- 配置系统级限流、并发、预算、保留周期。

### 普通用户

普通用户消费网关能力。

能力：

- 登录 Web 面板。
- 创建、禁用、删除自己的 API Key。
- 查看自己的调用记录、Token/请求量统计、错误率。
- 查看自己可用的模型列表。
- 查看自己的额度、限流和剩余额度。

### 下游 API 调用方

通过 API Key 调用网关。

能力：

- 使用 OpenAI-compatible 或 Codex-compatible API 调用模型。
- 接收普通响应或 SSE 流式响应。
- 在错误时获得尽量兼容上游格式的错误响应。

## 3. 一期功能范围

一期目标是做出可用的中转站聚合核心，而不是一次性做完整商业化平台。

必须包含：

- Rust 后端服务。
- React + shadcn/ui Web 面板。
- 管理员/普通用户登录。
- 用户 API Key 管理。
- 多上游配置。
- 上游健康检查。
- 基础路由策略。
- 兼容式请求代理。
- 非流式响应代理。
- SSE 流式响应代理。
- 请求日志与用量记录。
- 基础统计面板。
- SQLite 或 PostgreSQL 数据库支持，建议初版优先 SQLite，结构预留 PostgreSQL。

暂缓到二期：

- 复杂计费系统。
- 多租户组织。
- 发票/支付。
- 精细 RBAC。
- 插件化策略引擎。
- 分布式部署。
- 高级风控。

## 4. 总体架构

```text
                    ┌────────────────────────┐
                    │ React + shadcn Web UI   │
                    └───────────┬────────────┘
                                │ Admin/User API
                                ▼
┌──────────────┐       ┌────────────────────────┐
│ Downstream   │──────▶│ codex-gateway Backend  │
│ Codex Client │       │ Rust / Axum            │
└──────────────┘       └───────────┬────────────┘
                                   │
       ┌───────────────────────────┼───────────────────────────┐
       ▼                           ▼                           ▼
┌──────────────┐            ┌──────────────┐            ┌──────────────┐
│ Upstream A   │            │ Upstream B   │            │ Upstream C   │
│ Codex Relay  │            │ Codex Relay  │            │ Codex Relay  │
└──────────────┘            └──────────────┘            └──────────────┘

                 ┌────────────────────────┐
                 │ DB: users, keys, logs   │
                 └────────────────────────┘
```

建议后端模块：

- `api`: 管理面板和下游兼容 API 路由。
- `auth`: 登录、会话、API Key 鉴权、密码哈希。
- `proxy`: 请求转发、响应转发、SSE 流式处理。
- `routing`: 上游选择、模型映射、失败重试。
- `upstream`: 上游配置、健康检查、能力探测。
- `usage`: 用量估算、响应解析、请求记录。
- `storage`: 数据库实体、migration、查询。
- `config`: 环境变量和系统配置。
- `telemetry`: tracing、metrics、审计日志。

## 5. 推荐技术栈

后端：

- Rust 2024。
- `axum` 作为 HTTP 框架。
- `tokio` 异步运行时。
- `reqwest` 作为上游 HTTP 客户端。
- `sqlx` 作为数据库访问层。
- `serde` / `serde_json` 处理请求和透传 JSON。
- `tower` / `tower-http` 做中间件、CORS、超时、限流。
- `tracing` / `tracing-subscriber` 做日志。
- `argon2` 做密码哈希。
- `jsonwebtoken` 或服务端 session 做 Web 登录态。

前端：

- React。
- TypeScript。
- Vite。
- shadcn/ui。
- Tailwind CSS。
- TanStack Query。
- React Router。
- Recharts 或 Tremor 风格组件做基础图表。

数据库：

- 初版推荐 SQLite，便于单机部署。
- schema 设计避免 SQLite 特有写法，为 PostgreSQL 迁移保留空间。

## 6. 下游 API 设计

下游调用方通过 `Authorization: Bearer cgk_xxx` 使用网关。

一期建议提供两类入口：

### 兼容透传入口

```text
POST /v1/chat/completions
POST /v1/responses
POST /responses
POST /responses/compact
POST /v1/responses/compact
GET  /v1/models
```

处理原则：

- 请求体尽量原样透传。
- 响应体尽量原样返回。
- Header 只做必要过滤与改写。
- 支持 `stream: true` 的 SSE 流式返回。
- 下游看到的 `model` 可以是网关模型名，由网关映射到上游模型名。

真实 Codex CLI 流量观察显示，Codex 0.142.5 主要调用 `POST /responses`，并使用 Responses API 风格的 SSE 事件，如 `response.output_text.delta`、`response.function_call_arguments.delta` 和 `response.completed`。因此一期代理核心应优先保证 `/responses` 的流式透传和 usage 解析。

Codex 的远程上下文压缩也需要纳入设计。实测流量中观察到 `compaction_trigger` 作为 `/responses` input item 发出，并由 `/responses` 流返回 `compaction` output item；官方源码和文档还显示存在独立的 `/responses/compact` 端点。因此初版应支持 `/responses/compact` 的原样代理，即使当前上游未必一定使用该路径。

不再设计 `ANY /codex/{*path}` 作为一期入口。当前抓包和公开资料没有证明 Codex CLI 会调用这个路径；如果未来某个上游确实提供私有路径，再用显式 passthrough route 配置补充，而不是预设一个无证据的固定 `/codex` 前缀。

生图接口 `POST /v1/images/generations` 暂不纳入一期支持。实测 `gpt-image-2` 虽在模型列表中，但上游返回 `503 api_error: No available compatible accounts`；该接口不是 Codex CLI 的核心 coding flow。

## 7. 上游模型与路由

### 上游配置字段

上游最小字段：

- `name`: 展示名。
- `base_url`: 上游中转站地址。
- `api_key`: 上游密钥，加密存储或至少避免日志输出。
- `enabled`: 是否启用。
- `priority`: 优先级，数字越小越优先。
- `weight`: 权重，用于加权轮询。
- `timeout_ms`: 请求超时。
- `max_retries`: 最大重试次数。
- `health_check_path`: 健康检查路径。

### 模型映射

示例：

```text
下游模型名: codex-mini
上游 A: gpt-5-codex-mini
上游 B: codex-mini
上游 C: disabled
```

模型配置字段：

- `public_name`: 下游可见模型名。
- `upstream_name`: 上游真实模型名。
- `upstream_id`: 绑定上游。
- `enabled`: 是否启用。
- `visible_to_users`: 是否在 `/v1/models` 展示。
- `fallback_model_id`: 可选兜底模型。

### 一期路由策略

建议先实现 3 种：

- `priority`: 按优先级选择第一个健康上游。
- `weighted`: 在健康上游中按权重选择。
- `sticky_by_key`: 同一个 API Key 尽量固定到同一上游，减少行为抖动。

默认策略建议：

```text
priority + health check + retry next upstream
```

也就是优先走管理员指定的主上游，失败时自动尝试下一个健康上游。

### 重试原则

可以重试：

- 连接失败。
- 上游超时。
- HTTP 502/503/504。
- 上游明确返回临时错误。

不建议重试：

- 已开始向下游输出的流式请求。
- 400/401/403/404。
- 明显由请求内容导致的错误。

## 8. 代理行为

### 请求处理流程

```text
1. 解析 Authorization。
2. 校验 API Key 状态、用户状态、额度和限流。
3. 解析请求体，提取 model、stream、metadata。
4. 根据模型和策略选择上游。
5. 改写上游 URL、Authorization、必要 Header。
6. 转发请求。
7. 将响应状态码、Header、Body 或 SSE 事件返回给下游。
8. 异步或同步写入 request log 与 usage。
```

### Header 处理

下游到上游：

- 删除 `host`。
- 删除 `authorization`，替换为上游密钥。
- 保留 `content-type`、`accept`、`user-agent`、必要的 tracing header。
- 可选追加 `x-codex-gateway-request-id`。

上游到下游：

- 保留状态码。
- 保留 `content-type`。
- 对 SSE 保留 `text/event-stream`。
- 删除可能泄漏上游信息的敏感 Header。
- 可选追加 `x-codex-gateway-upstream`，管理员调试模式才返回。

### 流式响应

SSE 代理必须做到：

- 不缓存完整响应。
- 边读上游边写下游。
- 正确处理客户端断开。
- 正确记录结束状态。
- 在流已经开始后，不再切换上游。

Token 使用量：

- 如果上游最终返回 usage，记录真实 usage。
- 如果没有 usage，一期可记录请求数和字符数估算。
- 估算值必须标记 `usage_source = estimated`。

## 9. 数据库设计草案

### users

```text
id
email
password_hash
role                 admin | user
status               active | disabled
display_name
created_at
updated_at
last_login_at
```

### api_keys

```text
id
user_id
name
key_prefix
key_hash
status               active | disabled | revoked
last_used_at
expires_at
created_at
revoked_at
```

只保存 hash，不保存明文 key。创建时只展示一次。

### upstreams

```text
id
name
base_url
api_key_ciphertext
enabled
priority
weight
timeout_ms
max_retries
health_check_path
last_health_status   healthy | degraded | down | unknown
last_health_checked_at
created_at
updated_at
```

### models

```text
id
public_name
description
enabled
visible_to_users
created_at
updated_at
```

### upstream_models

```text
id
model_id
upstream_id
upstream_model_name
enabled
priority
weight
created_at
updated_at
```

### request_logs

```text
id
request_id
user_id
api_key_id
model_id
upstream_id
method
path
status_code
error_code
stream
prompt_tokens
completion_tokens
total_tokens
usage_source         upstream | estimated | unknown
input_chars
output_chars
latency_ms
started_at
finished_at
client_ip_hash
user_agent
```

### daily_usage

```text
id
date
user_id
api_key_id
model_id
upstream_id
request_count
error_count
stream_count
prompt_tokens
completion_tokens
total_tokens
latency_ms_sum
created_at
updated_at
```

可由 request_logs 聚合生成。一期也可以每次请求后 upsert，方便面板快速展示。

## 10. Web 面板设计

### 信息架构

管理员导航：

- Overview
- Requests
- Users
- API Keys
- Upstreams
- Models
- Settings

普通用户导航：

- Overview
- API Keys
- Requests
- Usage

### 管理员首页

核心卡片：

- 今日请求数。
- 今日 Token。
- 错误率。
- 平均延迟。
- 健康上游数量。

图表：

- 近 24 小时请求量。
- 近 7 天 Token 用量。
- 各模型请求占比。
- 各上游错误率。

列表：

- 最近错误请求。
- 上游健康状态。

### 上游管理页

能力：

- 新增上游。
- 编辑 base URL、密钥、优先级、权重、超时。
- 启用/禁用。
- 手动健康检查。
- 查看最近请求、错误率、延迟。

交互建议：

- 密钥输入框默认遮蔽。
- 保存前显示连接测试按钮。
- 健康状态使用明显 badge。

### 模型管理页

能力：

- 创建下游可见模型名。
- 配置模型到多个上游的映射。
- 设置默认上游顺序。
- 设置是否对普通用户可见。

### 用户 API Key 页

能力：

- 创建 API Key。
- 只展示一次明文。
- 显示 key prefix、创建时间、最后使用时间、状态。
- 禁用/删除 key。
- 按 key 查看用量。

## 11. 安全设计

基础要求：

- Web 登录密码使用 Argon2 哈希。
- API Key 使用随机高熵 token，数据库只存 hash。
- 上游 key 不写日志。
- 请求日志不保存完整 prompt 和 completion，默认只保存统计信息。
- 管理员可以开启短期 debug，但必须脱敏。
- 所有管理 API 需要 CSRF 或 Bearer/JWT 防护。
- CORS 默认只允许面板来源。
- 生产环境必须配置 `APP_SECRET`。

API Key 格式建议：

```text
cgk_live_{prefix}_{secret}
```

其中：

- `prefix` 用于快速定位。
- `secret` 只 hash 存储。

## 12. 配置设计

环境变量示例：

```text
CODEX_GATEWAY_BIND=0.0.0.0:8080
CODEX_GATEWAY_DATABASE_URL=sqlite://data/codex-gateway.db
CODEX_GATEWAY_APP_SECRET=change-me
CODEX_GATEWAY_PUBLIC_URL=http://localhost:8080
CODEX_GATEWAY_LOG_LEVEL=info
CODEX_GATEWAY_ADMIN_EMAIL=admin@example.com
CODEX_GATEWAY_ADMIN_PASSWORD=change-me-on-first-login
```

系统配置表可管理：

- 默认路由策略。
- 请求超时。
- 最大请求体大小。
- API Key 默认额度。
- request log 保留天数。
- 是否返回上游 debug header。

## 13. 错误格式

下游错误尽量保持兼容：

```json
{
  "error": {
    "message": "No healthy upstream available for model codex-mini",
    "type": "gateway_error",
    "code": "upstream_unavailable"
  }
}
```

建议错误码：

- `invalid_api_key`
- `disabled_api_key`
- `quota_exceeded`
- `rate_limited`
- `model_not_found`
- `upstream_unavailable`
- `upstream_timeout`
- `upstream_error`
- `gateway_internal_error`

## 14. 可观测性

日志：

- 每个请求生成 `request_id`。
- tracing span 包含 user_id、api_key_id、model、upstream、status、latency。
- 不记录敏感 header。

指标：

- 请求数。
- 错误数。
- 延迟分布。
- 上游健康状态。
- 模型用量。
- 用户用量。

一期可以先由数据库统计驱动面板，后续再加 Prometheus `/metrics`。

## 15. 部署形态

一期推荐单二进制部署：

```text
codex-gateway
├── embedded static web assets
├── SQLite database
└── config from env
```

开发模式：

- 后端：`cargo run`
- 前端：`pnpm dev`

生产模式：

- 前端构建为静态文件。
- Rust 后端内置或托管静态文件。
- 反向代理可选。

## 16. 一期里程碑

### M1: 后端基础

- Axum 服务启动。
- 配置加载。
- 数据库 migration。
- tracing。
- health endpoint。

### M2: 认证与用户

- 管理员初始化。
- Web 登录。
- 用户 CRUD。
- API Key 创建与校验。

### M3: 上游与模型

- 上游 CRUD。
- 模型 CRUD。
- 模型映射。
- 健康检查。

### M4: 代理核心

- `/v1/models`。
- `/responses`。
- `/responses/compact`。
- `/v1/chat/completions`。
- `/v1/responses`。
- `/v1/responses/compact`。
- 非流式转发。
- SSE 流式转发。
- 基础重试与失败切换。
- 明确不支持生图接口。

### M5: 用量与统计

- request_logs。
- daily_usage 聚合。
- 用户和管理员统计 API。

### M6: Web 面板

- 登录页。
- Overview。
- API Key 管理。
- 上游管理。
- 模型管理。
- 请求日志页。

## 17. 初版验收标准

功能验收：

- 管理员能添加两个上游。
- 管理员能配置一个下游模型映射到两个上游。
- 普通用户能创建 API Key。
- 使用该 API Key 调用 `/v1/chat/completions` 成功。
- `stream: true` 能正常返回 SSE。
- 主上游不可用时，非流式请求能 fallback 到备用上游。
- 请求结束后能在面板看到请求记录和基础用量。
- 禁用 API Key 后调用会被拒绝。

兼容验收：

- 下游请求体中的未知字段不会被网关误删。
- 上游响应中的未知字段不会被网关误删。
- 普通错误响应格式接近 OpenAI-compatible 风格。
- SSE 不发生整包缓存。

安全验收：

- 数据库不保存明文下游 API Key。
- 日志不输出上游 API Key。
- 普通用户无法查看其他用户请求记录。
- 普通用户无法访问管理员接口。

## 18. 关键设计取舍

### 为什么先做透传而不是强 schema

Codex 中转站之间可能存在细节差异。初版如果强行定义完整请求 schema，会更容易破坏兼容性。更好的做法是：

- 只解析必要字段，如 `model`、`stream`。
- 其他字段作为 JSON 原样透传。
- 只在明确需要时做模型名替换。

### 为什么请求内容默认不入库

中转站天然会处理敏感 prompt。初版只保存统计信息和错误摘要，既能做分析，也能降低泄漏风险。

### 为什么先 SQLite

项目早期单机部署最简单。`sqlx` + migration 可以保留迁移到 PostgreSQL 的路线，不把数据库选型变成初版阻力。

## 19. 二期方向

- 额度包和计费。
- 用户组和组织。
- 精细模型权限。
- Prompt/response 可选审计采样。
- Prometheus metrics。
- Webhook。
- 上游自动探活和熔断。
- 更完整的限流策略。
- 批量导入用户和上游。
- 多实例共享状态。
- PostgreSQL 官方支持。
