# Codex Loop Skill

[中文](README.md)

MCP server (`codex-mcp-server`) wrapping [Codex app-server](https://github.com/openai/codex), plus a distributable **Codex Loop Skill** for orchestrating multi-thread development workflows in Hermes.

**Quick start:** [Installation](#installation)

## Components

| Path | Description |
|------|-------------|
| `src/` | Rust MCP server (`codex-mcp-server`) |
| `skills/codex-loop/` | Agent skill source (workflow docs + MCP config template) |

## Installation

End-to-end setup for Hermes: install the skill package, register the bundled MCP server, verify, then reload tools in a session.

### Prerequisites

| Requirement | Notes |
|-------------|-------|
| Hermes | CLI + runtime; `hermes` on PATH |
| [codex CLI](https://github.com/openai/codex) | Required on the host; MCP talks to Codex app-server over stdio |
| Platform `.skill` package | Pre-built binary inside the package — no Rust toolchain needed for release install |

### Step 1: Download the skill package

Pick the archive matching your OS from [GitHub Releases](https://github.com/xinjiyuan97/codex-skill/releases):

| Platform | Release asset |
|----------|---------------|
| macOS Apple Silicon | `codex-loop-macos-aarch64.skill` |
| macOS Intel | `codex-loop-macos-x86_64.skill` |
| Linux x86_64 | `codex-loop-linux-x86_64.skill` |
| Windows x86_64 | `codex-loop-windows-x86_64.skill` |

### Step 2: Install the skill in Hermes

Install the `.skill` archive per Hermes documentation (import / install skill package). After install, note the skill root path — referred to below as `<skill-root>`.

The package includes:

- `SKILL.md` — orchestration workflow
- `assets/bin/codex-mcp-server` — platform-specific MCP binary
- `scripts/setup.sh` — MCP config helper
- `assets/mcp-config.example.yaml` — config template

### Step 3: Configure MCP (manual, one-time)

The agent **cannot** edit `~/.hermes/config.yaml`. Run these in your terminal:

```bash
cd <skill-root>
chmod +x scripts/setup.sh

# Check codex CLI + bundled binary
./scripts/setup.sh --verify
```

**Option A — Hermes CLI (recommended)**

```bash
./scripts/setup.sh --install
```

In the interactive tool picker, enable:

- Tools: `start`, `reply`, `process`, `archive`
- Resources: **on** (`project://`, `thread://`)
- Prompts: **off**

**Option B — Edit config manually**

```bash
./scripts/setup.sh --print-config
```

Merge the snippet into `~/.hermes/config.yaml` under `mcp_servers.codex`, or copy from `assets/mcp-config.example.yaml` and set `command` to the **absolute** path of `assets/bin/codex-mcp-server`.

### Step 4: Verify

```bash
hermes mcp test codex
./scripts/setup.sh --verify
```

Expected:

- [ ] `hermes mcp list` shows `codex` as enabled
- [ ] `hermes mcp test codex` succeeds
- [ ] After reload (step 5), agent tool list includes `mcp_codex_start`, `mcp_codex_reply`, `mcp_codex_process`, `mcp_codex_archive`

If verification fails, see `skills/codex-loop/references/troubleshooting.md`.

### Step 5: Reload MCP in Hermes

In an active Hermes session, run:

```
/reload-mcp
```

Or start a new session. **Codex Loop Skill** is ready when `mcp_codex_*` tools appear.

### Install from source (development)

For contributors or local builds without a release package:

```bash
git clone https://github.com/xinjiyuan97/codex-skill.git
cd codex-skill

cargo test
cargo build --release

# Package a local .skill (pick your platform name)
./scripts/package-skill.sh macos-aarch64 target/release/codex-mcp-server dist
# → dist/codex-loop-macos-aarch64.skill

# Install dist/*.skill in Hermes, then run Steps 3–5 above.
# Or symlink skill source and point MCP at the built binary:
ln -s "$(pwd)/skills/codex-loop" ~/.hermes/skills/codex-loop   # path per your Hermes setup
./skills/codex-loop/scripts/setup.sh --print-config
```

When using a dev build, set `command` in `config.yaml` to `target/release/codex-mcp-server` (absolute path).

## MCP Server

Exposes Codex thread lifecycle over MCP:

| Tool | Purpose |
|------|---------|
| `start` | Create a thread with `description` + `prompt` |
| `reply` | Continue an existing thread |
| `process` | Inspect execution trace and status |
| `archive` | Archive and remove a thread from listings |

Resources:

- `project://{project_id}` — list threads by working directory
- `thread://{project_id}/{thread_id}` — thread detail (local + remote state)

Requires the `codex` CLI on the host machine. The MCP binary communicates with Codex app-server over stdio.

### Local Development

```bash
cargo test
cargo build --release
./target/release/codex-mcp-server
```

Configure Hermes MCP manually, or use `skills/codex-loop/assets/mcp-config.example.yaml` as a template.

Environment variables:

| Variable | Values | Default |
|----------|--------|---------|
| `CODEX_MCP_APPROVAL_POLICY` | `approve`, `session`, `deny` | `approve` |

## Workflow

Hermes-oriented agent skill. Guides an orchestrator agent through:

1. Classify task → create git branch (`feature/`, `bugfix/`, `refactor/`)
2. `mcp_codex_start` Codex threads with full markdown requirements
3. Validate results → `mcp_codex_reply` on the same thread to fix issues
4. Split complex work across module-scoped threads
5. Track progress via `mcp_codex_process` and MCP resources

See `skills/codex-loop/SKILL.md` for the full workflow and `skills/codex-loop/examples.md` for examples.

Installation steps: [Installation](#installation).

### Package Skill Locally

```bash
cargo build --release
./scripts/package-skill.sh macos-aarch64 target/release/codex-mcp-server dist
# → dist/codex-loop-macos-aarch64.skill
```

## CI / Release

GitHub Actions workflow `.github/workflows/build-skill.yml`:

- Builds `codex-mcp-server` for macOS (aarch64 + x86_64), Linux, Windows
- Packages platform-specific `.skill` archives
- Creates a GitHub Release on `v*` tags

## Project Layout

```
.
├── src/                    # MCP server source
├── skills/codex-loop/      # Skill source
│   ├── SKILL.md
│   ├── examples.md
│   ├── scripts/setup.sh    # MCP config helper
│   ├── references/         # MCP setup, tools, troubleshooting
│   └── assets/
│       ├── mcp-config.example.yaml
│       └── bin/            # Populated at package time
├── scripts/package-skill.sh
└── .github/workflows/build-skill.yml
```

## License

MIT
