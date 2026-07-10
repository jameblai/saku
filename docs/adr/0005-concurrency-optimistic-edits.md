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

## Consequences

- Cross-thread / external file conflicts surface as tool errors the model can recover from (re-read, retry).
- Read-snapshot table is keyed by (Session, path) — or path with Session-scoped store.
- Hourglass reactions must be removed when the queued Run starts (or is cancelled).
