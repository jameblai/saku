# ADR 0010: Pi-style Session Compaction

## Status

Accepted

## Context

Disk-backed Sessions can grow beyond the Provider context window during long coding threads. Options included hard truncation, failing the Run, deferring, or pi-style summarization of older turns.

## Decision

Implement Compaction inspired by pi: when nearing the context limit, summarize older transcript into a compact entry; retain recent messages and always include Memory.

## Consequences

- Need token estimation / Provider limit awareness and a summarization path (likely an extra LLM call).
- Session Store must record compaction entries so restarts don’t revive pre-compact bulk naively.
- Tuning (thresholds, keep-recent budget) can live in `config.toml` with sensible defaults.
