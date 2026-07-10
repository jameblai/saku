# ADR 0008: Bash timeout is optional (pi-like)

## Status

Accepted

## Context

Pi’s bash tool has no default timeout; the model may pass `timeout` in seconds. A Discord bot could instead impose a default (e.g. 120s) for safety on a Raspberry Pi. The user prefers pi’s behavior: no harness default.

## Decision

- `bash` accepts optional `timeout` (seconds). If omitted, the command runs until exit, `stop`, or process failure.
- Users abort runaway commands with the `stop` Bot Command.

## Consequences

- Long installs/builds won’t die at 120s unless the model sets a timeout or the user stops.
- Greater reliance on `stop` and Authorised User vigilance; consider a configurable hard max later if needed.
