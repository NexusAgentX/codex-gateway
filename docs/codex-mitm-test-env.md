# Codex MITM 测试环境

本项目内置一套 Codex 透明 MITM 测试环境，位于 `infra/codex-mitm-lab`。

这个实验环境会在 Docker 容器内运行 Codex CLI 和透明 mitmproxy，将容器内 Codex 发出的 HTTPS 流量解密保存为 flow 文件。宿主机的 Codex 配置和认证文件会被同步到实验目录，但不会提交到本项目。

这套环境参考了 `/srv/ops/codex-mitm-lab`，但不依赖该目录。换机器 clone 本仓库后，只要本机有 Docker 和可用的 `~/.codex`，就可以直接启动。

## 文件结构

```text
infra/codex-mitm-lab/
├── Dockerfile
├── docker-compose.yml
├── scripts/
│   ├── entrypoint.sh
│   └── sync-codex-config.py
└── runtime/              # 本地运行时目录，已被 .gitignore 忽略
```

## 启动

默认使用 `https://ai.input.im` 作为 Codex base URL：

```bash
scripts/codex-mitm-lab.sh up
```

进入容器：

```bash
scripts/codex-mitm-lab.sh shell
```

容器内直接正常使用 Codex：

```bash
codex
```

## 分析

抓包文件位于：

```text
infra/codex-mitm-lab/runtime/flows/codex.mitm
infra/codex-mitm-lab/runtime/flows/codex.pcap
```

输出脱敏摘要：

```bash
scripts/codex-mitm-lab.sh analyze
```

只输出汇总，不逐条打印 flow：

```bash
scripts/codex-mitm-lab.sh analyze --limit 0
```

分析脚本只输出：

- host、method、path、status。
- 与兼容性有关的请求/响应 header。
- JSON 顶层字段、model、stream、usage、error。
- SSE 事件数量和事件字段。

分析脚本不会输出：

- Authorization。
- Cookie。
- API key。
- prompt 正文。
- completion 正文。

## 常用命令

```bash
scripts/codex-mitm-lab.sh sync
scripts/codex-mitm-lab.sh restart
scripts/codex-mitm-lab.sh logs
scripts/codex-mitm-lab.sh down
```

指定 Codex CLI 版本：

```bash
CODEX_VERSION=0.142.5 scripts/codex-mitm-lab.sh restart
```

指定 Docker Compose 项目名：

```bash
CODEX_MITM_COMPOSE_PROJECT=codex-gateway-mitm scripts/codex-mitm-lab.sh up
```

指定宿主机 Codex 配置目录：

```bash
CODEX_SOURCE_HOME="$HOME/.codex" scripts/codex-mitm-lab.sh sync
```

如果需要模拟部分辅助域名超时：

```bash
MITM_BLOCK_HOSTS="github.com api.github.com raw.githubusercontent.com chatgpt.com ab.chatgpt.com files.openai.com" \
  scripts/codex-mitm-lab.sh restart
```

不要把 `ai.input.im` 放进 `MITM_BLOCK_HOSTS`，否则模型请求也会被黑洞掉。
