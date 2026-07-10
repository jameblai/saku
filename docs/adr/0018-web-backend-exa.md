# ADR 0018: Web Backend abstraction; Exa search + extract first

## Status

Accepted

## Context

Coding Runs often need current public-web docs and page bodies. Issue #19 flagged Hermes-style web search/extract as a gap; Saku only had `bash`/curl. Pi-inspired reference tooling (`websearch` + local `webfetch`) informed the split, but **Provider** already means the LLM backend (ADR-0002), so a second pluggable web identity needed its own name and auth story. We also wanted room for multiple web vendors later without baking Exa into the Tool names.

## Decision

- Introduce **Web Backend** (not Provider): a pluggable public-web service for search and/or extract. Configure the active one in `config.toml` (`web_backend`, default `exa`).
- Ship **Exa** first via the **official REST API** (`/search`, `/contents`) with an API-key **Credential** in `auth.json`. On-ramp: `saku login exa` only — not part of Setup.
- Expose two Tools: **`web_search`** and **`web_extract`**. Register them **only** when a Credential exists for the configured Web Backend.
- Both Tools go through Exa (no local HTTP fetch / HTML→markdown pipeline in v1).
- `web_search`: `query`; optional `max_results` (default 8); optional `depth` `auto`|`fast`|`deep` (default `auto`, mapped to Exa `type`). Returns title/url/snippet-style hits — not inline page bodies.
- `web_extract`: single `url`; returns page text only (no highlights/summary modes). Hard-truncate both Tools at **30 000 characters** (same spirit as `find`/`grep`); no spill file.
- Future Web Backends may register the **same Tool names with different parameter schemas**; only one backend is active at a time.

## Considered options (rejected)

- **Local `webfetch`** for the second Tool — more code and SSRF surface; deferred while Exa `/contents` covers extract.
- **Exa via MCP gateway** (as in the pi reference’s personal endpoint) — extra hop and non-portable protocol; prefer public REST.
- **Always register web Tools** and error at call time — wastes model turns; omit until Credential exists.
- **Overloading Provider** for Exa — collides with LLM Provider / Credential language.
- **Truncate + temp spill** (pi-style) — skipped for v1; hard truncate only.
- **Search-with-contents / rich extract modes** — blurs the two-Tool split; text extract only for v1.

## Consequences

- Web capability is opt-in per host (`saku login exa`).
- Tool schemas are not guaranteed stable across Web Backends — document per backend.
- Swapping away from Exa later means a new Web Backend adapter; Tool names stay `web_search` / `web_extract`.
