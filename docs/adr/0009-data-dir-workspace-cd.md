# ADR 0009: Data Dir, configurable Workspace, soft confine + cd

## Status

Accepted

## Context

Earlier we treated Workspace as a fixed home directory with Memory inside it. The user wants Saku state under `~/.saku`, a configurable Workspace root (default home), soft confinement for file tools, unrestricted `bash` for the OS user, and a per-Session `cd` tool that cannot escape the Workspace — including against `..` and similar tricks.

## Decision

- **Data Dir** is `~/.saku` (`config.toml`, Credentials, Session Store, `MEMORY.md`, etc.).
- **Workspace** path is set in `config.toml`, defaulting to the bot user’s home directory.
- File tools (`read`/`edit`/`write`/`find`/`grep`/`ls`/`cd`) resolve paths with **canonicalization** (resolve `.` / `..` and symlinks) and **reject** any result that is not strictly inside the Workspace root.
- **`cd`** changes only that Session’s Working Directory; initial Working Directory is the Workspace root; target must be an existing directory inside the Workspace.
- **`bash`** starts in the Session Working Directory but is not path-jailed (soft confine). Real isolation remains OS user / container if needed.
- ~~File tools may always read/write exactly `~/.saku/MEMORY.md` (and enforce the 2200 cap on write) even if outside Workspace.~~ **Superseded by ADR 0020** (dedicated `memory` Tool; no file-tool hole).

## Consequences

- Path helper is security-critical and must be tested (traversal, symlink escape, absolute paths outside root). Memory allowlist exception removed — see ADR 0020.
- FFF index root follows Workspace.
- Working Directory is part of Session Store so it survives restarts.
