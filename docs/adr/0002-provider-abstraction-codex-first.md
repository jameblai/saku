# ADR 0002: Provider abstraction; Codex subscription first

## Status

Accepted

## Context

Saku needs LLM access with vision and tool calling. Pi supports many providers via a credential store and OAuth registry; ChatGPT Plus/Pro (Codex) uses OAuth against `auth.openai.com` and the Codex Responses API at `chatgpt.com/backend-api`. The user wants Codex sign-in for v1 and an abstraction so other subscriptions and API-key logins can be added later without rewriting the harness.

## Decision

- Introduce a Provider + Credential abstraction (login, refresh, resolve request auth) inspired by pi’s `CredentialStore` / OAuth provider registry.
- Ship **OpenAI Codex (ChatGPT subscription OAuth)** as the first concrete Provider.
- Do not hard-wire the harness to Codex-only types; additional Providers (Anthropic OAuth, API keys, etc.) plug in later.

## Consequences

- v1 implements Codex OAuth (prefer device-code for headless/RPi) and Codex Responses streaming/tool-calling.
- Auth storage is keyed by provider id with serialized refresh.
- Discord/bot UX for login is a separate decision from this API shape.
