# ADR 0014: Codebase shape

## Status

Accepted

## Context

Saku needs clear crate and type boundaries so the Harness stays testable without Discord, while CLI login and the Serenity bot remain thin adapters. Credential persistence should follow pi’s proven auth.json pattern under Saku’s Data Dir.

## Decision

### Crates (Cargo workspace)

| Crate | Responsibility |
|--------|----------------|
| `saku-harness` | Harness, Tools, Provider + Codex, Session Store, Credential store, Compaction, path jail, Memory helpers, `config.toml` types/load. No Serenity. |
| `saku-discord` | Serenity adapter: gateway events → Harness; threads, reactions, attachments, message chunking. |
| `saku-cli` | Host CLI UX: `login`, config-related commands that do not need Discord. |
| `saku` | Binary crate that wires the three; `saku` runs the bot, `saku login` delegates to `saku-cli`. |

### Credentials

- Path: `~/.saku/auth.json`
- Schema inspired by pi: map of id → `{ "type": "oauth", access, refresh, expires, ... }` or `{ "type": "api_key", "key": "..." }` (Provider or Web Backend)
- File mode `0600`; refresh serialized with a file lock
- Not required to be path-compatible with `~/.pi/agent/auth.json`

### Harness API (Session-centric)

- `Harness` owns shared config, Provider, tool registry, Credential access.
- `Harness::session(thread_id) -> Session` loads or creates from Session Store.
- `Session::run(UserTurn) -> RunHandle` starts or queues a Run; `UserTurn` is text plus optional images.
- `Session` exposes Bot Command operations: `stop`, `steer`, `set_model`, `set_effort`, and list helpers for model/effort.
- `RunHandle` exposes an async stream (or callback) of `RunEvent` (text deltas, tool start/end, status, finished, error, aborted).
- No Serenity / Discord types in this API.

### RunEvent vocabulary (rich / pi-like)

Emitted on `RunHandle` for adapters and tests:

- `RunStarted`, `Queued`, `Dequeued` (hourglass lifecycle)
- `TextDelta`, `ReasoningDelta` (Discord may omit reasoning from user-visible posts)
- `ToolStarted { name, args }`, `ToolProgress`, `ToolFinished { name, ok }`
- `AssistantFinished`, `RunFinished`, `RunError`, `RunAborted`

### Tool plug-in shape

- Dynamic async `Tool` trait: `name`, `description`, `parameters_schema` (JSON Schema), `execute(ctx, args: serde_json::Value, abort) -> ToolResult`.
- Shared `ToolContext`: Workspace root, Session Working Directory, Read Snapshots, Memory path allow, progress callback, abort.
- `ToolResult`: content parts for the model, `is_error`, optional structured `details` for adapters/logs (not necessarily sent to the model).
- Tool failures become error tool results to the Provider (pi-style), not uncaught panics.

### Provider trait

- `Provider::complete(Request) -> Stream<ProviderEvent>` (async).
- `Request`: system prompt, transcript messages, tool definitions, model id, Effort, abort.
- `ProviderEvent`: text/reasoning deltas, tool-call assembly, message completion, usage, errors.
- Harness-owned transcript roles (`user` / `assistant` / `tool_result` + content parts); Codex maps at the boundary.
- Codex v1 implementation lives **inside** `saku-harness` (`provider::codex`), including device-code Login helpers used by `saku-cli`. No separate `saku-codex` crate until a second Provider or size justifies extraction.
- Tests use a scripted fake `Provider`.

### Session Store format

- Append-only **JSONL** per Discord thread: `~/.saku/sessions/<thread_id>.jsonl`.
- **Pi-inspired, linear** (not pi’s `id`/`parentId` branch tree — Discord threads are linear).
- First line: session header (`type: "session"`, version, thread id, created timestamp, initial cwd/model/effort as useful).
- Entry types (one JSON object per line), including:
  - `message` — user / assistant / tool_result (content parts: text, image, tool calls as needed)
  - `model_change`, `effort_change` (pi’s `thinking_level_change`)
  - `cwd_change` — Session Working Directory
  - `read_snapshot` — path + hash/mtime for optimistic edits
  - `compaction` — summary + pointer to first kept message/entry id
- Replay rebuilds Session state; prefer append over rewrite.

### Config (`~/.saku/config.toml`)

Required:

- `discord_token`
- `authorized_user_ids` (Discord snowflakes as strings)

Optional with defaults:

- `command_prefix` = `"saku"`
- `workspace` = home directory (`~`)
- `data_dir` = `"~/.saku"`
- `default_model` = `"gpt-5.5"`
- `default_effort` = `"medium"`
- `web_backend` = `"exa"`

Tuning knobs (compaction thresholds, image max dimension, etc.) stay code defaults until needed.

### `saku-harness` module tree

```text
saku-harness/src/
  lib.rs
  types.rs          # ContentPart, transcript messages, RunEvent, ProviderEvent, …
  config.rs
  credentials.rs
  path.rs           # canonicalize + Workspace jail
  memory.rs
  compaction.rs
  session/          # store (JSONL), Session, RunHandle
  harness.rs
  provider/         # trait + codex/
  tools/            # registry + bash, read, edit, write, find, grep, ls, cd
```

`ContentPart`: `Text` | `Image` (bytes + mime) shared across user turns, tool results, and Provider requests.

## Consequences

- Integration tests and unit tests for agent behavior target `saku-harness` only.
- Adding a Provider does not require touching Serenity.
- Binary size / compile graph is larger than a single crate; acceptable for boundary clarity.
- Discord adapter maps reactions/messages onto `Session` / `RunHandle` methods and consumes `RunEvent`s for Discord output.
- New Tools are additive registrations; no harness enum churn.
- Streaming Provider enables Discord live updates without waiting for full completion.
- Session Store JSONL is append-mostly and inspectable on disk.
- Discord can ignore `ReasoningDelta` for display while tests/logs still see it.
- Config surface stays small for Pi installs.
- Module tree is a guide; files can split further if a module grows, without changing public API contracts above.
- Session JSONL mirrors pi’s entry *ideas* but stays linear for Discord threads.
