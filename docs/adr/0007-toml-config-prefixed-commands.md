# ADR 0007: TOML config; prefixed Discord commands

## Status

Accepted

## Context

Saku needs host-side configuration (Command Prefix, credentials paths, timeouts, allowlisted Discord ids, Workspace). Rust’s ecosystem standard for app config is TOML. Cancel/help/steer should be explicit text commands in a Session thread, not reaction-based cancel.

## Decision

- Use **TOML** for the Saku config file.
- Bot Commands are messages beginning with a configurable Command Prefix: `stop`, `help`, `steer <message>`.
- `stop` aborts the active Run in the Session thread where it was typed (kills bash, stops Provider stream).
- `steer` injects a mid-Run user directive into that Session (pi-style steering).
- `help` replies with available Bot Commands / brief usage.

## Consequences

- Prefix must be chosen to avoid colliding with normal chat (document a default in config example).
- `steer` is supported as specified in the glossary.
