# ADR 0001: Rust, pi-inspired (not pi-dependent)

## Status

Accepted

## Context

Saku needs a Discord coding agent with bash/read/edit tools, vision, a small footprint suitable for a Raspberry Pi, and strong reliability/memory-safety. The reference design is earendil-works/pi, which is a Node.js ≥22 TypeScript monorepo (`pi-agent-core`, `pi-ai`, `pi-coding-agent`).

Options considered:

1. Depend on pi packages and add a Discord I/O layer
2. Reimplement in Rust, copying pi’s architectural shape (loop, tools, events)
3. Reimplement in Go

## Decision

Implement Saku in Rust. Treat pi as an architectural reference (agent loop, tool semantics, truncation, content-vs-details split), not as a dependency.

## Consequences

- No reuse of pi’s TypeScript packages or session/JSONL formats unless we deliberately mirror them later
- Smaller deployable binary and better fit for constrained hosts
- We own the LLM provider client(s), Discord gateway integration, and tool implementations
- Higher initial build cost than wrapping pi
- Concrete crate choices are guidance in ADR 0013 (Serenity preferred; add deps as features land)
