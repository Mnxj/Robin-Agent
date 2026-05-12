# Robin（中文说明）

`Robin` 是一个用 Rust 编写的本地自托管 AI Agent 网关。单二进制、低内存、默认运行在你的机器上。

英文完整版文档请见 [README.md](README.md)。

## 项目简介

- 提供统一入口：CLI、Web Chat、WebSocket JSON-RPC。
- 可连接多种模型：Claude、GPT、Gemini、Qwen，以及任意 OpenAI 兼容接口。
- Agent 可调用内置工具与 MCP 远程工具，在本机执行文件、命令、网络检索等任务。
- 主要状态都保存在 `~/.robin/`，便于备份与迁移。

## 核心能力

### 接口与运行方式

- `robin chat`：命令行对话。
- `robin start`：启动网关，Web Chat 默认地址 `http://127.0.0.1:18789/chat`。
- WebSocket API：`ws://127.0.0.1:18789/ws`。
- macOS / Windows 托盘应用：后台守护并提供管理入口。

### Agent 与工具

- 支持多 Agent（不同模型、工作区、工具权限）。
- 支持子 Agent（通过 `task` 工具委派）。
- 常用工具包括：`read_file`、`write_file`、`edit_file`、`bash`、`web_fetch`、`web_search`、`browser`、`todo_write`、`load_skill`、`load_memory`。
- MCP 工具统一走同一权限策略（allow/deny）。

### 上下文、记忆与技能

- 会话持久化为 JSONL（按 `agent + session`）。
- 支持会话压缩（compaction）和超长工具输出裁剪/落盘（spill）。
- Memory 使用 Markdown 条目；每轮会做自动召回，并可通过 `load_memory` 按需加载全文。
- Skills 使用 Markdown + YAML Frontmatter；System Prompt 注入索引，正文通过 `load_skill` 按需加载并支持热更新。

## 安装与构建

### 方式 1：源码构建

要求 Rust 1.75+。

```bash
cargo build -p robin
cargo build --release -p robin
./scripts/build-all.sh
docker build -t robin:local .
```

首次建议执行：

```bash
./target/debug/robin onboard
```

### 方式 2：安装脚本

仓库已提供一键安装脚本：

```bash
bash install.sh
```

## 快速开始

```bash
./robin onboard
./robin chat
./robin start
./robin doctor
```

`robin chat` 默认会自动复用已启动网关会话。  
如果要强制本地独立运行，可使用：

```bash
./robin chat --no-gateway
```

## 常用命令

| 命令 | 说明 |
| --- | --- |
| `robin onboard` | 首次配置向导 |
| `robin start` | 启动网关 |
| `robin chat [agent]` | 进入交互式聊天 |
| `robin clear [agent]` | 清空本地 CLI 会话 |
| `robin sessions [agent]` | 查看会话列表 |
| `robin mcp login <id>` | MCP OAuth 登录 |
| `robin status` | 查询网关 Agent 状态 |
| `robin doctor` | 诊断检查 |
| `robin version` | 显示版本信息 |

## 关键配置文件

主配置路径：

- macOS/Linux：`~/.robin/robin.json5`
- Windows：`C:\Users\<you>\.robin\robin.json5`

最小配置示例：

```json5
{
  "providers": {
    "anthropic": { "kind": "anthropic", "api_key": "sk-ant-..." }
  },
  "agents": {
    "list": [
      { "id": "default", "name": "Robin", "model": "anthropic/claude-sonnet-4-5" }
    ]
  }
}
```

## 目录结构（简要）

```text
felix-rust/
├── Cargo.toml
├── README.md
├── README.zh-CN.md
├── install.sh
├── Dockerfile
├── .github/workflows/release.yml
├── scripts/
│   ├── build-all.sh
│   ├── publish.sh
│   ├── clean-tool-logs.py
│   ├── clean-chat.sh
│   └── smoke-skill-memory.sh
├── dist/
└── crates/
    ├── robin/
    ├── robin-app/
    └── robin-internal/
```

## 数据目录（运行时）

默认都在 `~/.robin/`：

- `robin.json5`：配置
- `sessions/`：会话历史
- `skills/`：技能文件
- `memory/entries/`：记忆条目
- `workspace-<agentId>/`：每个 Agent 的工作区
- `brain.db`：Cortex（SQLite）
- `cron-jobs.json`：动态定时任务

## 开发命令

```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all
./scripts/build-all.sh
TAG=v0.1.0 ./scripts/publish.sh
./scripts/smoke-skill-memory.sh
```

## Skills 与 Memory 接口

- Skills 管理接口：`GET/POST/DELETE /settings/api/skills*`
- Memory 管理接口：`GET/POST/DELETE /settings/api/memory*`
- 验证脚本：`./scripts/smoke-skill-memory.sh`（需要网关已启动）

## 安全与默认策略

- 默认仅监听 `127.0.0.1`。
- 支持网关 Bearer Token 鉴权。
- 文件工具默认受工作区边界限制。
- `bash` 支持 `deny / allowlist / full` 三档执行策略。

---
