# ADR 0019: Session-scoped Background Processes

## Status

Accepted

## Context

Agents often need long-running helpers (local HTTP servers, watchers) that must stay up after the Run that started them finishes. Foreground `bash` is unsuitable: it is aborted by `saku stop` / Run abort and uses `kill_on_drop(true)`. Users also need a way to inspect and stop those processes from Discord without starting a new Run, while keeping *start* agent-only. Issues #28–#30 specify the Tool and Bot Command surface.

## Decision

- Introduce **Background Process** as a Session-scoped concept (see `CONTEXT.md`).
- Agent Tools: `bg_start`, `bg_list`, `bg_logs`, `bg_stop`.
  - `bg_start`: spawn `bash -lc` in a new process group (`process_group(0)`), cwd frozen at call time, optional `settle` (default 2s), return PG-leader PID + short output snippet; soft cap **5 running** per Session; ~**1 MiB** in-memory ring buffer (drop oldest).
  - `bg_list`: running and exited entries (exited keep exit code; no prune).
  - `bg_logs`: tail with optional `lines` (default 100, hard-capped).
  - `bg_stop`: by PID or all; signal whole process group (SIGTERM → 5s → SIGKILL).
- Bot Commands (Authorised Users, no start): `saku bg`, `saku bg logs <pid>`, `saku bg stop <pid>`, `saku bg stop all`. Flat `saku help` documents them; `saku status` includes a one-line running/exited summary. `saku stop` remains Run-only and must not kill Background Processes.
- State lives on the in-memory `LiveSession` (not Session Store JSONL). Lost on bot restart.

## Considered options (rejected)

- **Reuse foreground `bash` with detach flags** — still tied to tool abort / `kill_on_drop`; process-group semantics unclear.
- **Persist Background Processes in Session Store** — restart recovery is valuable but out of scope for v1; OS PIDs are not stable across bot restarts without a supervisor.
- **Allow `saku bg start` via Bot Command** — keeps start agent-mediated (args, cwd, intent) and avoids a second start path.
- **Kill Background Processes on `saku stop`** — contradicts the “outlives the Run” purpose.

## Consequences

- Long-running servers survive Run end and `saku stop`; users stop them explicitly via Tools or `saku bg stop`.
- Cap of 5 running prevents unbounded process growth per Session.
- Bot restart orphans any still-running OS processes (no reattach in v1); document that limitation.
- Discord adapter stays parse-and-delegate; all process logic lives in `saku-harness`.
