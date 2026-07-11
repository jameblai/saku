# ADR 0023: Attribute all Session-owned Provider usage

## Status

Accepted

## Context

Session usage currently accounts for parent Run Provider turns but omits Provider work performed by Subagents and Goal Evaluators. This makes token and estimated-cost totals incomplete, especially when a Subagent overrides the Session model or several children run concurrently.

## Decision

Session Usage includes every Provider turn caused by the Session: parent Runs, Subagents, and Goal Evaluators. Each Provider-reported measurement is persisted immediately as a Usage Record containing only its Usage Source (`run`, `subagent`, or `goal_evaluator`), model, and token counts. Estimated USD cost is derived from those durable facts using the current model-rate table whenever Session state is built; cost is never persisted as fact. Usage is retained when the enclosing operation later fails, is aborted, or reaches a limit; failure to persist a Usage Record fails the enclosing Run rather than silently undercounting consumption.

`saku status` continues to show combined token and estimated-cost totals, followed by a compact cost breakdown for non-zero Usage Sources. Model attribution remains persisted for auditability but is not shown in the default status response.

Usage entries without an explicit source or model replay as `run` records and infer the model active at their position in the Session log. Cost is never read back from the log; all estimates use current rates.

## Consequences

Usage totals become complete and explainable across inner Harness activity. Session Store replay must remain backward-compatible, and parallel Subagents may append Usage Records in nondeterministic order; aggregation is therefore additive and must not depend on record ordering.
