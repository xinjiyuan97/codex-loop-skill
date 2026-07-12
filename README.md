# Codex Loop Skill

[English](README.en.md)

基于 [Codex app-server](https://github.com/openai/codex) 的 MCP 服务（`codex-mcp-server`），附带可分发的 **Codex Loop Skill**，用于在 Hermes 中编排多线程开发工作流（新功能、bugfix、refactor、PR review、批量 issue 修复）。

**快速开始：** [安装流程](#安装流程)

## 项目组成

| 路径 | 说明 |
|------|------|
| `src/` | Rust MCP 服务（`codex-mcp-server`） |
| `skills/codex-loop/` | Agent Skill 源码（工作流文档 + MCP 配置模板） |

## 安装流程

在 Hermes 中完整安装：导入 skill 包 → 注册 bundled MCP server → 验证 → 在会话中重载工具。

### 前置条件

| 依赖 | 说明 |
|------|------|
| Hermes | 需安装 CLI 与运行时，`hermes` 在 PATH 中 |
| [codex CLI](https://github.com/openai/codex) | 宿主机必须安装；MCP 通过 stdio 连接 Codex app-server |
| 对应平台的 `.skill` 包 | 包内已含编译好的二进制，Release 安装无需 Rust 工具链 |

### 步骤 1：下载 skill 包

从 [GitHub Releases](https://github.com/xinjiyuan97/codex-skill/releases) 选择与本机匹配的文件：

| 平台 | Release 资源 |
|------|-------------|
| macOS Apple Silicon | `codex-loop-macos-aarch64.skill` |
| macOS Intel | `codex-loop-macos-x86_64.skill` |
| Linux x86_64 | `codex-loop-linux-x86_64.skill` |
| Windows x86_64 | `codex-loop-windows-x86_64.skill` |

### 步骤 2：在 Hermes 中安装 skill

按 Hermes 文档导入 / 安装 `.skill` 包。安装完成后记下 skill 根目录路径，下文记为 `<skill-root>`。

包内包含：

- `SKILL.md` — 编排工作流
- `assets/bin/codex-mcp-server` — 对应平台的 MCP 二进制
- `scripts/setup.sh` — MCP 配置辅助脚本
- `assets/mcp-config.example.yaml` — 配置模板

### 步骤 3：配置 MCP（用户手动，一次性）

Agent **无法**修改 `~/.hermes/config.yaml`，请在终端执行：

```bash
cd <skill-root>
chmod +x scripts/setup.sh

# 检查 codex CLI 与 bundled 二进制
./scripts/setup.sh --verify
```

**方式 A — Hermes CLI（推荐）**

```bash
./scripts/setup.sh --install
```

在交互式工具选择界面中勾选：

- 工具：`start`、`reply`、`process`、`archive`
- Resources：**开启**（`project://`、`thread://`）
- Prompts：**关闭**

**方式 B — 手改 config.yaml**

```bash
./scripts/setup.sh --print-config
```

将输出合并进 `~/.hermes/config.yaml` 的 `mcp_servers.codex`，或参考 `assets/mcp-config.example.yaml`，把 `command` 改为 `assets/bin/codex-mcp-server` 的**绝对路径**。

### 步骤 4：验证

```bash
hermes mcp test codex
./scripts/setup.sh --verify
```

确认以下项：

- [ ] `hermes mcp list` 中有 `codex` 且为 enabled
- [ ] `hermes mcp test codex` 成功
- [ ] 完成步骤 5 重载后，Agent 工具列表含 `mcp_codex_start`、`mcp_codex_reply`、`mcp_codex_process`、`mcp_codex_archive`

失败时见 `skills/codex-loop/references/troubleshooting.md`。

### 步骤 5：在 Hermes 中重载 MCP

在当前 Hermes 会话执行：

```
/reload-mcp
```

或新开一个会话。当 `mcp_codex_*` 工具出现后，即可使用 **Codex Loop Skill**。

### 从源码安装（开发）

贡献者或本地构建、不使用 Release 包时：

```bash
git clone https://github.com/xinjiyuan97/codex-skill.git
cd codex-skill

cargo test
cargo build --release

# 打包本地 .skill（按平台选择 os_name）
./scripts/package-skill.sh macos-aarch64 target/release/codex-mcp-server dist
# → dist/codex-loop-macos-aarch64.skill

# 在 Hermes 中安装 dist/*.skill，再执行上文步骤 3–5。
# 或直接链接 skill 源码，MCP 指向本地编译产物：
ln -s "$(pwd)/skills/codex-loop" ~/.hermes/skills/codex-loop   # 路径以你的 Hermes 配置为准
./skills/codex-loop/scripts/setup.sh --print-config
```

开发模式下，`config.yaml` 的 `command` 应指向 `target/release/codex-mcp-server`（绝对路径）。

## MCP 服务

通过 MCP 暴露 Codex 线程生命周期管理：

| 工具 | 用途 |
|------|------|
| `start` | 创建线程，传入 `description` + `prompt` |
| `reply` | 在已有线程上继续对话 |
| `process` | 查看执行轨迹和状态 |
| `archive` | 归档并从列表中移除线程 |

资源（Resources）：

- `project://{project_id}` — 按工作目录列出线程
- `thread://{project_id}/{thread_id}` — 线程详情（本地状态 + 远程 Codex 数据）

宿主机需安装 `codex` CLI。MCP 二进制通过 stdio 与 Codex app-server 通信。

### 本地开发

```bash
cargo test
cargo build --release
./target/release/codex-mcp-server
```

手动配置 Hermes MCP，或以 `skills/codex-loop/assets/mcp-config.example.yaml` 为模板。

环境变量：

| 变量 | 可选值 | 默认值 |
|------|--------|--------|
| `CODEX_MCP_APPROVAL_POLICY` | `approve`, `session`, `deny` | `approve` |

## 工作流

面向 **Hermes** 的 Agent Skill，通过 `mcp_codex_*` 工具编排 Codex 开发循环：

1. 分类任务 → 创建 git 分支（`feature/`、`bugfix/`、`refactor/`）
2. 用完整 markdown 需求调用 `mcp_codex_start` 创建 Codex 线程
3. 校验结果 → 在同一 thread 上 `mcp_codex_reply` 修正
4. 复杂任务按模块拆分为多个 thread；PR review、worktree 并行修复、批量 review 见 SKILL.md
5. 通过 `mcp_codex_process` 和 MCP resources 跟踪进度

完整工作流见 `skills/codex-loop/SKILL.md`，示例见 `skills/codex-loop/examples.md`。

安装步骤见 [安装流程](#安装流程)。

### 本地打包 Skill

```bash
cargo build --release
./scripts/package-skill.sh macos-aarch64 target/release/codex-mcp-server dist
# → dist/codex-loop-macos-aarch64.skill
```

## CI / 发布

GitHub Actions 工作流 `.github/workflows/build-skill.yml`：

- 为 macOS（aarch64 + x86_64）、Linux、Windows 编译 `codex-mcp-server`
- 打包各平台 `.skill` 压缩包
- 打 `v*` tag 时自动创建 GitHub Release

## 目录结构

```
.
├── src/                    # MCP 服务源码
├── skills/codex-loop/      # Skill 源码
│   ├── SKILL.md
│   ├── examples.md
│   ├── scripts/setup.sh    # MCP 配置辅助脚本
│   ├── references/         # MCP 配置、工具列表、排错
│   └── assets/
│       ├── mcp-config.example.yaml
│       └── bin/            # 打包时注入二进制
├── scripts/package-skill.sh
└── .github/workflows/build-skill.yml
```

## 许可证

MIT
