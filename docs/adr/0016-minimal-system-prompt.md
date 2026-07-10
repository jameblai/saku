# ADR 0016: Minimal system prompt (iterate in code)

## Status

Accepted

## Context

The Harness needs a system prompt every Run (plus injected Memory). Grilling the full prose before scaffolding would delay the project without changing architecture.

## Decision

Ship a **minimal fixed system prompt** in `saku-harness`, including roughly:

- Identity: Saku, Discord-hosted coding agent for an Authorised User
- Current Workspace root and Session Working Directory
- Injected Memory contents (or empty Memory guidance)
- Tool-use norms: use `find`/`grep` before blind `bash` search; `read` before `edit`; Memory only for durable important facts (2200 cap); respect path confine
- Brief Bot Command awareness is optional (commands are handled outside the model)

Iterate prompt wording in code/PRs; no separate prompt-grilling gate.

## Consequences

- First vertical slice can boot with a short prompt constant or template.
- Prompt changes do not require ADR updates unless behavior policy changes (e.g. Memory rules).
