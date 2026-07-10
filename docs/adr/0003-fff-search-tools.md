# ADR 0003: Coding tools include search via fff-search

## Status

Accepted

## Context

v1 needs pi-like coding tools: bash, read, edit, write, plus search. The `@ff-labs/pi-fff` extension shows the intended FFF integration for agents: replace subprocess `find`/`grep` with in-process FFF `fileSearch`/`grep`, keep `ls` separate, and leave multi-grep off by default. Workspace is the bot home directory (`/home/saku`); pi-fff already enables home-dir scanning for that case.

## Decision

- Tool surface: `bash`, `read`, `edit`, `write`, `find`, `grep`, `ls`.
- Implement `find` and `grep` with the native `fff-search` crate, following pi-fff semantics (frecency, smart-case, pagination, weak-match capping, fuzzy fallback for grep, reject wildcard-only patterns).
- Keep `ls` as a thin readdir tool (not FFF).
- Do not ship multi-grep in v1.
- Index the Workspace with FFF AI mode and home-dir scanning enabled; refuse filesystem-root indexing unless explicitly opted in.

## Consequences

- Long-lived bot process holds an FFF index for `/home/saku`.
- Tool names match common agent training (`find`/`grep`), not `fffind`/`ffgrep`.
- Dependency on `fff-search` (and its LMDB/frecency stack) instead of shelling out to `rg`/`fd`.
