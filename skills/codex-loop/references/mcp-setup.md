# Codex MCP Setup (Hermes)

持久集成：用户在 `~/.hermes/config.yaml` 注册 `codex` MCP server，Skill 只教 Agent 使用 `mcp_codex_*` 工具。

## 架构

```
Hermes Agent  --mcp_codex_*-->  codex-mcp-server (bundled binary)
                                      |
                                      v
                               Codex app-server (codex CLI)
```

- **stdio** 传输，无 HTTP/OAuth
- 二进制：`<skill-root>/assets/bin/codex-mcp-server`
- 宿主机需安装 `codex` CLI

## 用户手动步骤（Agent 无法代劳）

Agent **不能**修改 `~/.hermes/config.yaml`。以下由用户在本机终端完成。

### 方式 A：Hermes CLI（推荐）

```bash
cd <skill-root>
chmod +x scripts/setup.sh
./scripts/setup.sh --verify

# 交互式添加（需在 TUI 中勾选工具）
./scripts/setup.sh --install

hermes mcp test codex
```

在 Hermes 会话中执行 `/reload-mcp`。

工具勾选建议（白名单）：

```yaml
tools:
  include: [start, reply, process, archive]
  resources: true
  prompts: false
```

### 方式 B：手改 config.yaml

参考 `assets/mcp-config.example.yaml`：

```yaml
mcp_servers:
  codex:
    command: "/absolute/path/to/codex-loop/assets/bin/codex-mcp-server"
    args: []
    env:
      CODEX_MCP_APPROVAL_POLICY: "approve"
    tools:
      include: [start, reply, process, archive]
      resources: true
      prompts: false
```

保存后：

```bash
hermes mcp test codex
# Hermes 会话: /reload-mcp
```

### 方式 C：仅打印配置片段

```bash
./scripts/setup.sh --print-config
```

## 环境变量

| 变量 | 写入位置 | 默认 | 说明 |
|------|----------|------|------|
| `CODEX_MCP_APPROVAL_POLICY` | `config.yaml` → `mcp_servers.codex.env` | `approve` | 命令/文件变更审批 |

敏感项不要写进 Skill 或对话；无 API Key 需求（本地 stdio）。

## 相关 Skill

配置与 MCP 通用问题可查 **`native-mcp`** skill（`skill_view native-mcp`）。

## 可自动化 vs 需手动

| 可自动化 | 需用户手动 |
|----------|-----------|
| `setup.sh --verify` | 安装 codex CLI |
| `setup.sh --print-config` | `hermes mcp add` 工具勾选（curses） |
| `hermes mcp test`（用户执行） | `/reload-mcp` 或新开会话 |
| | 下载对应平台 `.skill` 包 |
