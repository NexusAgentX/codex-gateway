# Codex Gateway 协议文档

本文档描述 codex-gateway 当前实现所兼容的 Codex 中转站上游协议，以及协议判断所依据的抓包证据。

资料来源：

- 本仓库 MITM 实验室抓包：`infra/codex-mitm-lab`
- 实测上游：`https://ai.input.im`
- 实测 Codex CLI：`0.142.5`
- OpenAI Codex 源码：
  - `codex-rs/codex-api/src/common.rs`
  - `codex-rs/codex-api/src/endpoint/responses.rs`
  - `codex-rs/codex-api/src/endpoint/responses_websocket.rs`
  - `codex-rs/core/src/client.rs`
  - `codex-rs/core/src/compact_remote*.rs`
  - `codex-rs/model-provider-info/src/lib.rs`
  - `codex-rs/protocol/src/models.rs`

本文档不记录 prompt、completion、Authorization、Cookie、API key 或任何用户内容。

## 0. 文档边界

本文档只定义 Codex 客户端、codex-gateway、Codex 中转站上游之间的 wire protocol 兼容事实：

- endpoint、method、header、请求/响应字段。
- SSE 事件形态。
- usage/error 在协议里的位置。
- 哪些路径有抓包或源码证据，哪些没有。
- MITM 抓包证据摘要。

已实现的产品功能、数据库行为、路由策略、重试策略和 Web 面板以 `docs/features.md` 为准。若两者冲突：

- 协议形态、路径和字段以本文档为准。
- 产品、架构、落库和运营行为以当前功能说明为准。

## 1. 协议结论

Codex 现在的主协议是 Responses API。

Codex 源码中 `wire_api = "chat"` 已被移除，当前 provider wire protocol 只保留：

```text
wire_api = "responses"
```

当前已经支持：

```text
POST /responses
POST /v1/responses
POST /responses/compact
POST /v1/responses/compact
GET  /v1/models
```

当前不支持：

```text
POST /v1/chat/completions
POST /v1/images/generations
WebSocket /responses
POST /realtime/calls
POST /memories/trace_summarize
ANY  /codex/{*path}
```

原因：当前 Codex provider wire protocol 只保留 Responses；Chat
Completions 不是实测 Codex CLI 核心路径。实测 `gpt-image-2`
虽出现在模型列表中，但调用返回 `503 api_error: No available compatible
accounts`，生图也不是 Codex CLI 核心路径，因此当前未实现。

当前未实现：

```text
ANY /codex/{*path}
```

原因：抓包、源码和公开资料都没有证明 Codex CLI 通过该固定路径访问模型 provider。未来如有私有上游路径，应以显式 passthrough route 配置补充，而不是预设无证据路径。

## 2. Base URL 与路径

Codex provider 配置中的 `base_url` 会作为模型 API 根地址。

本次实验配置：

```toml
[model_providers.OpenAI]
base_url = "https://ai.input.im"
wire_api = "responses"
requires_openai_auth = true
```

实测 Codex CLI 调用：

```text
POST https://ai.input.im/responses
```

因此 codex-gateway 给下游 Codex CLI 使用时，推荐让下游配置：

```toml
base_url = "https://gateway.example.com"
wire_api = "responses"
```

网关应兼容两种路径风格：

```text
/responses
/v1/responses
```

对上游转发时应按上游配置决定路径：

- 上游 base URL 是 `https://host`：转发到 `https://host/responses`
- 上游 base URL 是 `https://host/v1`：转发到 `https://host/v1/responses`

同理适用于 `/responses/compact`。

## 3. 鉴权

下游到网关：

```http
Authorization: Bearer cgk_...
```

网关到上游：

```http
Authorization: Bearer <upstream_key>
```

处理规则：

- 下游 key 只用于网关鉴权，不透传给上游。
- 上游 key 只用于上游请求，不返回给下游，不写日志。
- 请求日志中最多保存 key prefix、user id、upstream id。

## 4. 必要 Header

下游 Codex CLI 实测请求头：

```http
Accept: text/event-stream
Authorization: Bearer ...
Content-Type: application/json
User-Agent: codex-tui/0.142.5 (...)
```

网关转发到上游时：

- 必须设置上游 `Authorization`。
- 应保留 `Accept: text/event-stream`。
- 应保留 `Content-Type: application/json`。
- 可以保留 `User-Agent`，也可以追加/替换为网关标识。
- 必须删除 hop-by-hop header，如 `connection`、`keep-alive`、`transfer-encoding`、`upgrade`。

Codex 源码还定义了这些可能出现的 header：

```text
OpenAI-Beta
x-codex-installation-id
x-codex-turn-state
x-codex-turn-metadata
x-codex-parent-thread-id
x-codex-window-id
x-openai-memgen-request
x-openai-subagent
x-responsesapi-include-timing-metrics
x-codex-beta-features
x-openai-internal-codex-responses-lite
```

当前实现策略：

- 对未知 `x-codex-*`、`x-openai-*`、`openai-*` header 默认透传，除非明确敏感。
- `x-codex-turn-state` 可能用于上游粘性路由，必须透传。
- 不依赖这些 header 做网关业务逻辑。

## 5. `POST /responses`

这是 Codex CLI 的核心请求。

实测 31 次模型请求全部是：

```text
POST /responses
stream: true
Accept: text/event-stream
response Content-Type: text/event-stream
```

### 5.1 请求体顶层字段

实测 `/responses` 请求均包含：

```json
{
  "model": "gpt-5.5",
  "instructions": "...",
  "input": [],
  "tools": [],
  "tool_choice": "auto",
  "parallel_tool_calls": true,
  "reasoning": {
    "effort": "xhigh"
  },
  "store": false,
  "stream": true,
  "include": ["reasoning.encrypted_content"],
  "prompt_cache_key": "...",
  "text": {
    "verbosity": "low"
  },
  "client_metadata": {}
}
```

Codex 源码中的 `ResponsesApiRequest` 字段：

```text
model
instructions
input
tools
tool_choice
parallel_tool_calls
reasoning
store
stream
stream_options
include
service_tier
prompt_cache_key
text
client_metadata
```

兼容要求：

- 网关只轻解析 `model`、`stream`、`client_metadata`。
- 其他字段原样透传。
- 不强行校验 `input`、`tools`、`reasoning` 的完整 schema。
- 不记录 `instructions`、`input`、`tools`、`prompt_cache_key`、`encrypted_content` 明文。

### 5.2 固定值观察

本次实测：

```text
model: gpt-5.5
stream: true
store: false
parallel_tool_calls: true
tool_choice: auto
reasoning.effort: xhigh
text.verbosity: low
include: reasoning.encrypted_content
```

这些是观察值，不是协议常量。网关不能写死。

### 5.3 client_metadata

实测字段：

```text
x-codex-window-id
session_id
turn_id
x-codex-installation-id
thread_id
x-codex-turn-metadata
```

源码还会在 WebSocket 请求中加入 trace 信息：

```text
ws_request_header_traceparent
ws_request_header_tracestate
x-codex-ws-stream-request-start-ms
```

兼容要求：

- 原样透传 `client_metadata`。
- 日志中可以记录是否存在、字段名集合、session/thread hash。
- 不记录 `x-codex-turn-metadata` 原文。

## 6. Response Item 类型

`input` 与 SSE `item` 都是 Responses API item。

实测 input item 类型：

```text
message
function_call
function_call_output
reasoning
compaction_trigger
compaction
```

实测 output item 类型：

```text
message
function_call
reasoning
compaction
```

源码中 `ResponseItem` 还支持：

```text
additional_tools
agent_message
local_shell_call
tool_search_call
tool_search_output
custom_tool_call
custom_tool_call_output
web_search_call
image_generation_call
context_compaction
```

兼容要求：

- `ResponseItem` 必须按 JSON 原样透传。
- 未知 item type 不能导致请求失败。
- 网关统计时只按 `type` 计数，不解析正文。

## 7. Tools

实测 tool 类型：

```text
function
custom
tool_search
web_search
```

源码支持：

```text
function
namespace
tool_search
web_search
custom
```

`function` 工具通常含：

```json
{
  "type": "function",
  "name": "...",
  "description": "...",
  "strict": true,
  "parameters": {}
}
```

`custom` 工具通常含：

```json
{
  "type": "custom",
  "name": "...",
  "description": "...",
  "format": {
    "type": "...",
    "syntax": "...",
    "definition": "..."
  }
}
```

兼容要求：

- 不把工具限制为 OpenAI Chat Completions 的 function schema。
- `custom`、`tool_search`、`web_search` 必须原样透传。
- 工具 schema 可能很大，日志只记录工具数量和 type 分布。

## 8. SSE 响应

`POST /responses` 返回：

```http
Content-Type: text/event-stream
```

每个事件通常是：

```text
data: {json}

```

本次没有观察到：

```text
data: [DONE]
```

因此不能依赖 `[DONE]` 判断结束。

实测事件类型：

```text
response.created
response.in_progress
response.output_item.added
response.function_call_arguments.delta
response.function_call_arguments.done
response.content_part.added
response.output_text.delta
response.output_text.done
response.content_part.done
response.output_item.done
response.completed
keepalive
```

源码中 `ResponseEvent` 还会抽象出：

```text
SafetyBuffering
ServerModel
ModelVerifications
TurnModerationMetadata
ServerReasoningIncluded
ToolCallInputDelta
ReasoningSummaryDelta
ReasoningSummaryDone
ReasoningContentDelta
ReasoningSummaryPartAdded
RateLimits
ModelsEtag
```

兼容要求：

- SSE 必须边读边写，不能整包缓存。
- 网关可以旁路解析 SSE 做统计，但解析失败不能影响透传。
- 不重排、不合并、不过滤事件。
- 不假设所有 200 SSE 都有 `response.completed`。
- 客户端断开后应取消上游请求。

## 9. usage 统计

usage 在 `response.completed` 事件中：

```json
{
  "type": "response.completed",
  "response": {
    "id": "resp_...",
    "status": "completed",
    "usage": {
      "input_tokens": 0,
      "input_tokens_details": {
        "cached_tokens": 0,
        "cache_write_tokens": 0
      },
      "output_tokens": 0,
      "output_tokens_details": {
        "reasoning_tokens": 0
      },
      "total_tokens": 0
    }
  }
}
```

实测 29 个完整 completed 事件中均包含：

```text
input_tokens
input_tokens_details
output_tokens
output_tokens_details
total_tokens
```

兼容要求：

- 只在看见 `response.completed.response.usage` 时记录真实 token。
- 未看见 completed 时：
  - `stream_completed = false`
  - `usage_source = unknown`
  - `status = partial` 或 `stream_interrupted`
- 记录 `upstream_response_id = response.id`。
- 记录 `upstream_status = response.status`。

## 10. 远程压缩

Codex 有两种压缩路径。

### 10.1 `/responses` 内联压缩

本次实测观察到：

```text
flow 63: POST /responses
input tail: compaction_trigger
response item.type: compaction

flow 64: POST /responses
input contains: compaction
```

源码 `compact_remote_v2_attempt.rs` 也会把：

```json
{"type": "compaction_trigger"}
```

追加到 `input`，然后仍走 `/responses` 流式请求。

兼容要求：

- `/responses` 必须原样支持 `compaction_trigger`。
- SSE 中 `item.type = compaction` 必须原样透传。

### 10.2 独立 compact endpoint

源码 `client.rs` 定义：

```text
/responses/compact
```

这是 unary 请求，不是 SSE 流。

源码中的 `CompactionInput` 字段：

```text
model
input
instructions
tools
parallel_tool_calls
reasoning
service_tier
prompt_cache_key
text
```

网关应支持：

```text
POST /responses/compact
POST /v1/responses/compact
```

处理规则：

- 请求 JSON 原样透传。
- 响应 JSON 原样返回。
- 不按 SSE 处理。
- 超时可以比普通非流式请求更长。
- 日志不记录 compact 内容，只记录状态、延迟、模型、错误码。

## 11. `GET /v1/models`

实测：

```text
GET /v1/models
```

返回：

```json
{
  "object": "...",
  "data": [
    {
      "id": "gpt-5.5",
      "display_name": "gpt-5.5",
      "type": "model",
      "created_at": "2024-01-01T00:00:00Z"
    }
  ]
}
```

实测模型列表会动态变化，曾从 9 个变为 10 个。

兼容要求：

- 支持模型列表刷新。
- 聚合多个上游时按 `id` 去重或保留来源映射。
- 保留上游额外字段，如 `display_name`、`type`。
- 不假设 OpenAI 标准字段完整存在。

## 12. 生图接口

实测请求：

```text
POST /v1/images/generations
```

请求字段：

```text
model
prompt
size
quality
output_format
```

实测结果：

```text
gpt-image-2 -> 503 api_error: No available compatible accounts
gpt-image-1 -> 404 model_not_found
```

当前实现结论：

- 不实现生图支持。
- 不在默认模型能力中标记 image generation 可用。
- 后续如支持，应作为普通 JSON 透传接口实现，并保留上游错误。

## 13. WebSocket Responses

源码支持 Responses over WebSocket。

关键点：

```text
endpoint: responses
OpenAI-Beta: responses_websockets=2026-02-06
message type: response.create
```

WebSocket 请求体基于 `ResponsesApiRequest`，额外字段：

```text
previous_response_id
generate
client_metadata
```

实测本次没有观察到 WebSocket，因为当前实验链路走 HTTP SSE。

当前实现结论：

- 不要求实现 WebSocket。
- 如果下游 Codex 开启 `supports_websockets`，可能会尝试 `wss://.../responses`。
- 下游 provider 不应启用 `supports_websockets`。
- 后续支持 WebSocket 时，需要透明代理双向 text frame，并保留 `response.create` 消息结构。

## 14. Realtime 与 Memories

源码还定义：

```text
POST /realtime/calls
POST /memories/trace_summarize
```

本次抓包未观察到这些 endpoint。

当前实现结论：

- 不作为 Codex LLM 聚合网关核心范围。
- 不实现。
- 如果未来 Codex CLI 的常规 coding flow 开始调用，再纳入协议。

## 15. 辅助域名

实测 Codex CLI 还访问：

```text
ab.chatgpt.com POST /otlp/v1/metrics
api.github.com
github.com
codeload.github.com
raw.githubusercontent.com
registry.npmjs.org
chatgpt.com
files.openai.com
```

这些不是模型 provider base URL，不属于 codex-gateway 当前代理范围。

用途包括：

- 版本检查。
- 插件仓库下载。
- OTel metrics。
- 文件下载。
- ChatGPT 插件接口。

## 16. 错误格式

实测错误示例：

```json
{
  "error": {
    "message": "No available compatible accounts",
    "type": "api_error"
  }
}
```

```json
{
  "error": {
    "message": "Model \"gpt-image-1\" is not supported by any configured account in this group",
    "type": "model_not_found"
  }
}
```

协议兼容要求：

- 上游有响应时，尽量原样返回状态码和 JSON。
- 网关自己的鉴权、额度、限流错误使用 OpenAI-compatible `error` 包装。
- 不把上游 `503 api_error` 改写成 `gateway_internal_error`。

## 附录 A. MITM 抓包证据摘要

采集环境：

```text
lab: infra/codex-mitm-lab
Codex CLI: 0.142.5
upstream base_url: https://ai.input.im
flows: 75
```

抓包只保留协议形态、字段结构和统计信息，不记录 prompt、completion、Authorization、Cookie 或 API key。

### A.1 endpoint 分布

```text
ai.input.im GET  /v1/models              3
ai.input.im POST /responses              31
ai.input.im POST /v1/images/generations  9
```

其他被 Codex CLI 访问的域名包括 `ab.chatgpt.com`、`api.github.com`、`github.com`、`raw.githubusercontent.com`、`registry.npmjs.org`、`chatgpt.com`、`files.openai.com`。这些不是 provider base URL，不属于 codex-gateway 当前代理范围。

### A.2 `/responses` 样本

31 次 `/responses` 请求全部满足：

```text
method: POST
path: /responses
stream: true
request Accept: text/event-stream
request Content-Type: application/json
response Content-Type: text/event-stream
model: gpt-5.5
```

请求体顶层字段集合：

```text
client_metadata
include
input
instructions
model
parallel_tool_calls
prompt_cache_key
reasoning
store
stream
text
tool_choice
tools
```

观察到的 input item 类型：

```text
message
function_call
function_call_output
reasoning
compaction_trigger
compaction
```

观察到的 tool 类型：

```text
function
custom
tool_search
web_search
```

### A.3 SSE 样本

观察到的 SSE 事件类型：

```text
response.created
response.in_progress
response.output_item.added
response.function_call_arguments.delta
response.function_call_arguments.done
response.content_part.added
response.output_text.delta
response.output_text.done
response.content_part.done
response.output_item.done
response.completed
keepalive
```

未观察到 Chat Completions 风格的：

```text
data: [DONE]
```

单次请求最大观察到 979 个 SSE 事件。有 2 次 `/responses` flow 为 `200 text/event-stream`，但抓到 0 个事件，可能是客户端中断或 flow 截断。

### A.4 远程压缩样本

本次抓包没有观察到独立的：

```text
POST /responses/compact
```

但观察到了 `/responses` 内联压缩：

```text
flow 63: input tail contains compaction_trigger, response item.type = compaction
flow 64: input contains compaction
```

独立 `/responses/compact` 来自 Codex 源码和公开 API 形态，因此作为兼容入口保留。

### A.5 `/v1/models` 样本

模型列表曾从 9 个变为 10 个，说明上游模型列表会动态变化。复测观察到的 model id：

```text
codex-auto-review
gemma-4
glm-4.7-flash
glm-5.2
gpt-5.3-codex-spark
gpt-5.4
gpt-5.4-mini
gpt-5.5
gpt-image-2
kimi-k2.7-code
```

model item 观察到的字段：

```text
created_at
display_name
id
type
```

### A.6 `/v1/images/generations` 样本

初始抓包观察到一次：

```text
POST /v1/images/generations
```

后续同一实验环境复测：

```text
gpt-image-2 + full params     -> 503 api_error: No available compatible accounts
gpt-image-2 + minimal params  -> 503 api_error: No available compatible accounts
gpt-image-2 + quality low     -> 503 api_error: No available compatible accounts
gpt-image-1                   -> 404 model_not_found
```

因此当前实现不把生图作为 Codex 核心协议支持项。
