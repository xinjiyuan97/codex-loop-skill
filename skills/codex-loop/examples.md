# Codex Loop Skill — Examples

Hermes 中调用 **`mcp_codex_*`** 工具（server `codex`）。MCP 配置见 [references/mcp-setup.md](references/mcp-setup.md)。

## Example 1: Single bugfix

**User request**: "Fix login redirect loop when session expires"

### Actions

1. Branch: `git checkout -b bugfix/login-redirect-loop`
2. `mcp_codex_start` with:

```json
{
  "description": "Fix login redirect loop on session expiry",
  "prompt": "# Fix login redirect loop on session expiry\n\n## Type\nbugfix\n\n..."
}
```

3. Validate: run auth tests, manually verify redirect chain.
4. If redirect still loops → `mcp_codex_reply`:

```markdown
The redirect loop persists. In `src/middleware/auth.ts`, the session check
still redirects to /dashboard when `returnUrl` is set. Fix: preserve returnUrl
through the login flow and only redirect once.
```

5. `mcp_codex_archive` when done.

---

## Example 2: Multi-module feature

**User request**: "Add CSV export for reports"

### Decomposition

| Thread | Branch | `description` |
|--------|--------|---------------|
| T1 | `feature/csv-export-api` | CSV export REST endpoint |
| T2 | `feature/csv-export-ui` | CSV export button and download UI |

All on branch `feature/csv-export` (single branch, two threads).

### T1 mcp_codex_start (abbreviated)

```json
{
  "description": "CSV export REST endpoint",
  "prompt": "# CSV Export API\n\n## Type\nfeature\n\n## Scope\nImplement `GET /api/reports/:id/export` returning `text/csv` stream.\n..."
}
```

### T2 mcp_codex_start (after T1 validated)

```json
{
  "description": "CSV export button and download UI",
  "prompt": "# CSV Export UI\n\n## Type\nfeature\n\n## Prerequisite\nAPI endpoint at `GET /api/reports/:id/export` is implemented.\n..."
}
```

---

## Example 3: Refactor

**User request**: "Extract payment logic from OrderService into PaymentService"

1. Branch: `refactor/extract-payment-service`
2. Single thread (one cohesive refactor):

```json
{
  "description": "Extract PaymentService from OrderService",
  "prompt": "# Extract PaymentService from OrderService\n\n## Type\nrefactor\n\n## Context\nOrderService has grown to 800 lines; payment logic (lines 200-450) should be isolated.\n..."
}
```

3. If tests fail after refactor → `mcp_codex_reply` with specific failing test names and expected vs actual.

---

## Example 4: Async parallel threads with resource tracking

**User request**: "Build notification API and UI in parallel"

### Setup

Branch: `feature/notification-system`

### Spawn two async threads

```json
// T1 — API (block=false)
{ "description": "Notification REST API", "prompt": "...", "cwd": "/path/to/project", "block": false }

// T2 — UI (block=false)
{ "description": "Notification inbox UI", "prompt": "...", "cwd": "/path/to/project", "block": false }
```

### Monitor via project resource

读取 MCP resource：`project://{project_id}`（server `codex`）。

Poll until both threads show `status: "completed"`, or use `mcp_codex_process` per thread:

```json
{ "thread_id": "t-api" }
// → { "status": "running", "process": [...] }
```

### On completion

1. Validate each thread's output independently.
2. Fix issues via `mcp_codex_reply` on the same `thread_id`.
3. `mcp_codex_archive` both threads when done.

---

## Example 5: Progress inspection mid-run

Thread `t-api` has been running for several minutes.

### Check lightweight status

```json
{ "thread_id": "t-api" }
```

Response highlights:
- `status: "running"` — still working
- `process` last step: `{ "kind": { "item_completed": { "item": "command_execution" } }, "summary": "cargo test" }`

### Check full detail

读取 MCP resource：`thread://{project_id}/t-api`

Read `local.process` for the execution trace and `remote` for Codex conversation history.

### Act on status

| status | next step |
|--------|-----------|
| `running` | wait, re-poll |
| `waiting_approval` | check approval policy |
| `completed` | validate acceptance criteria |
| `failed` | `mcp_codex_reply` with error context from `error` field |
