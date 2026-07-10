# ADR 0012: Default config values

## Status

Accepted

## Context

New installs need sensible defaults so Saku runs with minimal TOML editing.

## Decision

- Command Prefix default: `saku` (commands look like `saku stop`, `saku model …`)
- New Session defaults: model `gpt-5.5`, Effort `medium`
- Workspace default: bot user home directory
- Data Dir: `~/.saku`
- Web Backend default: `exa`
- Release Channel default: `stable`

## Consequences

- Ship an example `config.toml` documenting allowlisted Discord user ids, bot token, and overrides.
