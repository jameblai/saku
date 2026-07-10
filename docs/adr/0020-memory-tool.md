# ADR 0020: Dedicated `memory` Tool (no file-tool hole)

## Status

Accepted (supersedes the Memory path-allowlist bullet in ADR 0009)

## Context

Memory lived at `~/.saku/MEMORY.md` with a file-tool allowlist so `read`/`edit`/`write` could touch it outside the Workspace. In practice the model treated “remember this” as chat: a clear remember signal produced an Answer Message and no disk write, so nothing survived across Sessions. Progress also had nothing Memory-shaped to show.

## Decision

- Add a core **`memory` Tool**: one argument `content` (string); **full replace** of Memory; **empty allowed** (clears). Enforce the 2,200 character cap on that Tool only.
- **Remove** the Memory path allowlist from file tools — `read`/`edit`/`write` (and other path-resolved tools) treat `MEMORY.md` like any path outside the Workspace (no special-case error).
- **Prompt only** (no harness NLP): instruct the model to call `memory` when retaining facts across Sessions, and never claim it remembered unless that call succeeded. Keep ADR 0016’s minimal-prompt posture; iterate wording in code.

## Consequences

- Progress shows Memory updates as a normal Tool line.
- ADR 0009’s confine story is simpler (no Memory exception in the path jail).
- Claiming “I’ll remember” without a Tool call remains possible; we accept that over brittle auto-write.
