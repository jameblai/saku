# ADR 0023: Attribute all Session-owned Provider usage

## Status

Accepted

## Context

Session usage currently accounts for parent Run Provider turns but omits Provider work performed by Subagents and Goal Evaluators. This makes token and estimated-cost totals incomplete, especially when a Subagent overrides the Session model or several children run concurrently.

## Decision

Session Usage includes every Provider turn caused by the Session: parent Runs, Subagents, and Goal Evaluators. Each Provider-reported measurement is persisted immediately as a Usage Record containing its Usage Source (`run`, `subagent`, or `goal_evaluator`), model, token counts, and estimated cost. Usage is retained when the enclosing operation later fails, is aborted, or reaches a limit; failure to persist a Usage Record fails the enclosing Run rather than silently undercounting consumption.

`saku status` continues to show combined token and estimated-cost totals, followed by a compact cost breakdown for non-zero Usage Sources. Model attribution remains persisted for auditability but is not shown in the default status response.

Historical usage entries without explicit source or model replay as `run` records, infer the model active at their position in the Session log, and retain their stored cost rather than being repriced.

## Consequences

Usage totals become complete and explainable across inner Harness activity. Session Store replay must remain backward-compatible, and parallel Subagents may append Usage Records in nondeterministic order; aggregation is therefore additive and must not depend on record ordering.
