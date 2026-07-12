# Troubleshooting (Hermes)

## `mcp_codex_*` 工具不出现

1. `hermes mcp list` — `codex` 应为 enabled
2. `hermes mcp test codex` — 连接成功
3. Hermes 会话执行 `/reload-mcp` 或新开 session
4. 检查 `config.yaml` 中 `tools.include` 是否含四个工具名

## `hermes mcp test codex` 失败

| 现象 | 处理 |
|------|------|
| binary not found | `command` 改为 bundled 二进制**绝对路径** |
| codex CLI missing | 用户安装 codex 并加入 PATH |
| exec format error | 下载匹配平台的 `.skill` 包 |

## 工具调用失败

| 现象 | 处理 |
|------|------|
| unknown thread_id | `mcp_codex_process` 或读 `project://` resource |
| waiting_approval | 检查 `CODEX_MCP_APPROVAL_POLICY` |
| TurnFailed | 读 `error`，同 thread `mcp_codex_reply` |

## 配置改了不生效

- 改 `~/.hermes/config.yaml` 后必须 `/reload-mcp`
- Agent **不能**改 config；让用户在终端处理

## Agent 常见误用

- 调用 `start` 而非 `mcp_codex_start`
- 未验证 MCP 就绪就开始编排
- 完整需求写入 `description` 而非 `prompt`
- 修正时新建 thread 而非 `mcp_codex_reply`
- 假设 Agent 能跑 `hermes mcp add` 的交互步骤

## 日志

- Hermes MCP stderr：`~/.hermes/logs/mcp-stderr.log`（如存在）
- 更多 MCP 排错见 **`native-mcp`** skill
