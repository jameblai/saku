# ADR 0005: Per-Session concurrency; optimistic file edits

## Status

Accepted

## Context

Workspace is a shared home directory. Parallel Runs across Discord threads are useful, but unchecked parallel `edit`/`write` can corrupt files. Users also want clear busy feedback without chat spam.

## Decision

- Allow at most one active Run per Session (thread). Further messages in that Session are queued; react with an hourglass on the waiting message — no textual “queued” reply.
- Separate Sessions may Run concurrently.
- `edit` fails unless the file was `read` earlier in the **Session** and is unchanged since that read.
- `write` may create a non-existent path; if the path exists, the same read-snapshot rule as `edit` applies.
- Snapshots persist across Runs in the same thread so the model is not forced to re-read unchanged files (avoids transcript bloat). Hash mismatch still catches cross-Session and external edits.
- Subagents use child-local, in-memory snapshots for their own read→edit safety. Those snapshots disappear with the child and neither persist to the Session Store nor authorize later parent edits.

## Consequences

- Cross-thread / external file conflicts surface as tool errors the model can recover from (re-read, retry).
- Read-snapshot table is keyed by (Session, path) — or path with Session-scoped store.
- Each Subagent has a separate ephemeral snapshot table, including edit Subagents; snapshot capability never crosses the parent/child boundary.
- Hourglass reactions must be removed when the queued Run starts (or is cancelled).
