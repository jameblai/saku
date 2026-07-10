# ADR 0013: Suggested Rust crate guide (Serenity)

## Status

Accepted (guidance — not a hard requirement)

## Context

Saku is a Rust Discord coding agent. Implementers (human or agent) need a starting map of crates, but pinning an exhaustive `Cargo.toml` up front creates unused dependencies. Discord choice: Serenity.

## Decision

### Discord

Prefer **Serenity** for gateway + HTTP (mentions, threads, reactions, message edits, attachments). Do **not** add Poise unless slash commands are explicitly required later — Bot Commands are plain message parsing (`saku …`).

### Suggested crates (add when the feature lands)

| When you build… | Consider |
|-----------------|----------|
| Async runtime | `tokio` |
| Config | `serde`, `toml` |
| CLI (`saku`, `saku login`) | `clap` |
| Errors / logs | `thiserror` and/or `anyhow`; `tracing`, `tracing-subscriber` |
| Paths / Data Dir | `dirs`, `dunce` |
| Session ids / snapshots | `uuid` or `ulid`; `blake3` or `sha2` |
| Codex HTTP + OAuth | `reqwest` (rustls); PKCE helpers (`sha2`, `rand`, `base64`, `url`) or `oauth2` if it fits; SSE as needed |
| Vision resize | `image` (and optionally `fast_image_resize`); `base64`; `mime_guess` |
| Search tools | `fff-search` |
| Temp / scratch | `tempfile` |

Twilight remains an acceptable alternative if Serenity proves a poor fit; switching does not require revisiting product decisions in `CONTEXT.md`.

### Dependency discipline

- **Do not** pre-install the full table in an empty project.
- Add a crate in the same change that first uses it.
- Drop or avoid crates that remain unused after a feature ships.
- Prefer std (`tokio::process`, `std::fs::canonicalize`, etc.) when enough.

## Consequences

- `Cargo.toml` grows with the product; reviews should flag speculative deps.
- Codex client remains custom code on top of `reqwest` — no mandated LLM SDK crate.
- Binary size will still reflect `fff-search`’s transitive tree once search tools are added.
