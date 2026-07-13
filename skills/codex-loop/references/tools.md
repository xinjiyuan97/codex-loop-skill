# Codex MCP Tools & Resources (Hermes)

MCP server 名：`codex`（`~/.hermes/config.yaml`）

Hermes 注册后的工具名带前缀 **`mcp_codex_`**。Agent 只调用下列名称，不要用裸 MCP 名。

## Tools

| Hermes 工具名 | MCP 原名 | 用途 |
|---------------|----------|------|
| `mcp_codex_start` | `start` | 新建 thread |
| `mcp_codex_reply` | `reply` | 同 thread 继续 |
| `mcp_codex_cancel` | `cancel` | 中断进行中的 turn |
| `mcp_codex_process` | `process` | 查看执行轨迹 / 轮询 |
| `mcp_codex_archive` | `archive` | 归档 thread |

### `mcp_codex_start`

| 参数 | 必填 | 默认 | 说明 |
|------|------|------|------|
| `description` | yes | — | 简短摘要，供编排 Agent 判断与 project 列表 |
| `prompt` | yes | — | 完整 markdown 需求 |
| `cwd` | no | server cwd | 项目绝对路径 |
| `block` | no | `true` | `false` 异步执行 |
| `model` | no | — | 模型覆盖 |
| `sandbox` | no | `danger-full-access` | 沙箱模式 |

### `mcp_codex_reply`

| 参数 | 必填 | 默认 |
|------|------|------|
| `thread_id` | yes | — |
| `prompt` | yes | — |
| `block` | no | `true` |

### `mcp_codex_cancel`

中断 thread 当前正在执行的 turn。成功后本地状态变为 `interrupted`，并在 process trace 中追加 `cancelled` step。

| 参数 | 必填 | 默认 | 说明 |
|------|------|------|------|
| `thread_id` | yes | — | 必须存在 active turn |

返回 `thread_id`、`cancelled` 和被中断的 `turn_id`。

### `mcp_codex_process`

| 参数 | 必填 | 默认 | 说明 |
|------|------|------|------|
| `thread_id` | yes | — | — |
| `round` | no | `3` | 仅 **进行中**（`starting` / `running` / `waiting_approval`）时生效：返回最近 N 轮 trace；**已完成**（`completed` / `failed` / `interrupted`）时忽略，只返回最后一轮。`0` 会被规范化为 `1` |

`process` 返回 `status`: `starting` | `running` | `waiting_approval` | `completed` | `failed` | `interrupted`

响应额外包含 `total_rounds`（总轮数）与 `rounds_included`（本次返回的轮数）。

### `mcp_codex_archive`

| 参数 | 必填 | 默认 | 说明 |
|------|------|------|------|
| `thread_id` | yes | — | — |

## Resources

需在 config 中 `tools.resources: true`。

| URI | 用途 |
|-----|------|
| `project://{project_id}` | 列出项目下所有 thread |
| `thread://{project_id}/{thread_id}` | 本地 trace + 远程对话 |

通过 Hermes MCP resource API 读取（详见 `native-mcp`）。`project_id` 由 `cwd` 哈希生成。

## Verification 工具清单

Agent 工具列表中应出现：

- `mcp_codex_start`
- `mcp_codex_reply`
- `mcp_codex_cancel`
- `mcp_codex_process`
- `mcp_codex_archive`

以及 resource 相关能力（若已启用）。
