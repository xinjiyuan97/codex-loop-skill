#!/usr/bin/env bash
# Codex Loop MCP setup helper for Hermes.
# Automates: dependency checks, config snippet, hermes mcp add/test when available.
# Manual: codex CLI install, tool whitelist in curses UI, /reload-mcp in session.
set -euo pipefail

SKILL_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_NAME="codex-mcp-server"
IS_WINDOWS=false
case "$(uname -s 2>/dev/null || echo unknown)" in
  MINGW* | MSYS* | CYGWIN* | Windows*) IS_WINDOWS=true ;;
esac
if $IS_WINDOWS; then
  BIN_NAME="codex-mcp-server.exe"
fi

BIN_PATH="$SKILL_ROOT/assets/bin/$BIN_NAME"
MCP_NAME="codex"
DEFAULT_POLICY="${CODEX_MCP_APPROVAL_POLICY:-approve}"
HERMES_CONFIG="${HOME}/.hermes/config.yaml"

usage() {
  cat <<EOF
usage: $0 [--print-config] [--install] [--verify]

  --print-config   Print ~/.hermes/config.yaml snippet (default)
  --install        Run 'hermes mcp add codex' when hermes CLI is available
  --verify         Check codex CLI, binary, and 'hermes mcp test codex'

Environment:
  CODEX_MCP_APPROVAL_POLICY   approve | session | deny (default: approve)
EOF
}

check_codex_cli() {
  if ! command -v codex >/dev/null 2>&1; then
    echo "error: codex CLI not found in PATH" >&2
    echo "install: https://github.com/openai/codex" >&2
    return 1
  fi
  echo "ok: codex CLI -> $(command -v codex)"
}

check_binary() {
  if [[ ! -f "$BIN_PATH" ]]; then
    echo "error: bundled binary missing: $BIN_PATH" >&2
    echo "hint: install from .skill release or build via package-skill.sh" >&2
    return 1
  fi
  if [[ ! -x "$BIN_PATH" ]] && ! $IS_WINDOWS; then
    chmod +x "$BIN_PATH" 2>/dev/null || true
  fi
  echo "ok: MCP binary -> $BIN_PATH"
}

print_config() {
  cat <<EOF
# Append to ${HERMES_CONFIG}
mcp_servers:
  ${MCP_NAME}:
    command: "${BIN_PATH}"
    args: []
    env:
      CODEX_MCP_APPROVAL_POLICY: "${DEFAULT_POLICY}"
    tools:
      include: [start, reply, process, archive]
      resources: true
      prompts: false
EOF
}

install_via_hermes() {
  if ! command -v hermes >/dev/null 2>&1; then
    echo "error: hermes CLI not found; use --print-config and edit ${HERMES_CONFIG} manually" >&2
    return 1
  fi

  check_codex_cli || return 1
  check_binary || return 1

  echo "running: hermes mcp add ${MCP_NAME} ..."
  hermes mcp add "$MCP_NAME" \
    --command "$BIN_PATH" \
    --env "CODEX_MCP_APPROVAL_POLICY=${DEFAULT_POLICY}"

  cat <<EOF

Manual steps (cannot be automated):
  1. In the tool picker, enable: start, reply, process, archive
  2. Enable resources for project:// / thread:// listings
  3. Run: hermes mcp test ${MCP_NAME}
  4. In Hermes session: /reload-mcp
EOF
}

verify() {
  local ok=true
  check_codex_cli || ok=false
  check_binary || ok=false

  if command -v hermes >/dev/null 2>&1; then
    if hermes mcp list 2>/dev/null | grep -q "$MCP_NAME"; then
      echo "ok: hermes mcp list contains \"$MCP_NAME\""
    else
      echo "warn: hermes mcp list missing \"$MCP_NAME\"" >&2
      ok=false
    fi
    if hermes mcp test "$MCP_NAME" >/dev/null 2>&1; then
      echo "ok: hermes mcp test $MCP_NAME"
    else
      echo "warn: hermes mcp test $MCP_NAME failed" >&2
      ok=false
    fi
  else
    echo "warn: hermes CLI not found; skip mcp list/test" >&2
  fi

  $ok
}

main() {
  local action="print"
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --print-config) action="print" ;;
      --install) action="install" ;;
      --verify) action="verify" ;;
      -h | --help) usage; exit 0 ;;
      *) echo "unknown option: $1" >&2; usage; exit 1 ;;
    esac
    shift
  done

  case "$action" in
    print)
      check_binary || exit 1
      print_config
      echo ""
      echo "Then: hermes mcp test ${MCP_NAME} && /reload-mcp in Hermes session"
      ;;
    install)
      install_via_hermes
      ;;
    verify)
      verify || exit 1
      echo "verification passed"
      ;;
  esac
}

main "$@"
