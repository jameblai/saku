# ADR 0011: Model and Effort Bot Commands; Codex model allowlist

## Status

Accepted

## Context

Users need to switch Codex models and reasoning effort from Discord without editing config mid-thread. Effort levels differ by model (e.g. `max` on GPT-5.6 family only). Carrying an old effort across a model switch can select an unsupported level.

## Decision

- Bot Commands:
  - `model` — list supported models
  - `model <id>` — select model; Effort resets to that model’s default
  - `model <id> <effort>` — select model and Effort (Effort must be valid for that model; otherwise reject / fall back to default — prefer reject with help text)
  - `effort` — list Effort levels for the current model
  - `effort <level>` — set Effort for the current model
- v1 Codex allowlist: `gpt-5.5`, `gpt-5.6-sol`, `gpt-5.6-terra`, `gpt-5.6-luna`, `gpt-5.4-mini`
- Model and Effort are **per Session**. `config.toml` supplies defaults for newly created Sessions.
- On model change without explicit Effort, set Effort to the model default (pi-like default: `medium` unless we define per-model defaults later)
- Supported Effort sets follow pi’s `getSupportedThinkingLevels` rules for these models (`max` only on 5.6.*)

## Consequences

- Persist model + Effort in the Session Store.
- `config.toml` holds `model` / `effort` defaults for new Sessions only.
- Vision requires image-capable models; all allowlisted models support image input per pi’s Codex catalog.
