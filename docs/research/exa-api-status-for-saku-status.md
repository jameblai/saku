# Exa API: account / usage / billing surfaces for `saku status`

**Date:** 2026-07-11  
**Question:** With an Exa API key (`x-api-key` against `https://api.exa.ai`), what account, usage, credits, plan, or rate-limit information is available that would be useful in Saku’s Discord `status` command (analogous to live Codex Plan Usage)?  
**Sources:** Official Exa docs (`docs.exa.ai` / `exa.ai/docs`), published OpenAPI specs, first-party GitHub SDKs (`exa-labs/exa-py`, `exa-labs/exa-js`). No third-party blogs.

---

## Executive summary

Exa is **pay-as-you-go credits**, not a Codex-style “plan quota remaining” product. **Remaining team credit balance is dashboard-only** — no public API returns balance. There **is** a Team Management Admin API that returns **historical spend per API key** and **per-key budget / rate-limit metadata**, plus a cheap Websets **team probe** that validates a key and returns concurrency — but **not** credits or search QPS remaining. Response headers do **not** document `X-RateLimit-*`; only `X-Request-Id` appears in the public OpenAPI.

---

## Auth model (what Saku already has)

| Fact | Source |
| --- | --- |
| Search/Contents use `POST https://api.exa.ai/search` and `/contents` with `x-api-key` | [Search API Reference](https://docs.exa.ai/reference/search-api-guide-for-coding-agents), [Contents API Reference](https://docs.exa.ai/reference/contents-api-guide-for-coding-agents) |
| Keys are created/managed in the dashboard | [dashboard.exa.ai/api-keys](https://dashboard.exa.ai/api-keys), Contents auth note linking there |
| Saku’s Web Backend uses this REST shape | [ADR 0018](../adr/0018-web-backend-exa.md) |

---

## What *is* available via API

### 1. Team Management API (Admin) — usage & key metadata

**Base URL:** `https://admin-api.exa.ai/team-management`  
**Auth:** `x-api-key: YOUR-SERVICE-KEY` (OpenAPI: “Service API key for team authentication”)  
**Spec:** [Team Management API Spec](https://docs.exa.ai/team-management-spec.yaml) · index: [OpenAPI Specification](https://docs.exa.ai/reference/openapi-spec)

| Endpoint | Returns (documented) | Source |
| --- | --- | --- |
| `GET /api-keys` | All team keys: `id`, `name`, `rateLimit` (QPS), `budgetCents`, `isOverBudget` (list); optional filter by `api_key_id` | [List API Keys](https://docs.exa.ai/reference/team-management/list-api-keys) |
| `GET /api-keys/{id}` | Same metadata for one key + `teamId`, `createdAt` | [Get API Key](https://docs.exa.ai/reference/team-management/get-api-key) |
| `GET /api-keys/{id}/usage` | **Authoritative billing spend** for a key over a period: `total_cost_usd`, `cost_breakdown[]` (`price_name`, `quantity`, `amount_usd`), `period`, `team_id`, `api_key_name` | [Get API Key Usage](https://docs.exa.ai/reference/team-management/get-api-key-usage) |
| `POST/PUT/DELETE /api-keys…` | Create/update/delete keys (name, `rateLimit`, `budgetCents`) | [Create](https://docs.exa.ai/reference/team-management/create-api-key) / [Update](https://docs.exa.ai/reference/team-management/update-api-key) |

**Usage endpoint details** ([Get API Key Usage](https://docs.exa.ai/reference/team-management/get-api-key-usage)):

- Default window: last **30 days**; `start_date` / `end_date` ISO 8601.
- Lookback capped at **6 months (180 days)**; older → `400`.
- Docs explicitly call this “cost data from Exa's billing system” / “authoritative view of what you're being billed.”
- Zero usage → `total_cost_usd: 0`, possibly empty `cost_breakdown`.
- Auth key must be **same team** as the requested key ID.

**Caveats for Saku:**

- Host is **`admin-api.exa.ai`**, not `api.exa.ai`.
- Docs call the credential a **“service API key”** / `YOUR-SERVICE-KEY`. They do **not** clearly state that every dashboard search key is interchangeable with a service key; OpenAPI only says “Service API key for team authentication” ([team-management-spec.yaml](https://docs.exa.ai/team-management-spec.yaml)). Treat “regular Exa key works as service key” as **likely but not explicitly guaranteed** until verified live.
- Usage requires the key’s **UUID** (`/api-keys/{id}/usage`). List returns IDs but **not** the secret material, so a bot that only stores the secret cannot map “this key” → UUID without an extra stored ID, a single-key team heuristic, or a naming convention.
- Documented create/list/get schemas **do not include the secret key string** ([Create API Key](https://docs.exa.ai/reference/team-management/create-api-key) OpenAPI response) — secrets appear to be dashboard-oriented for humans.
- **No remaining credit balance** field anywhere in this API (confirmed by scanning [team-management-spec.yaml](https://docs.exa.ai/team-management-spec.yaml): `budget`/`usage` present; `balance`/`credit` absent).

Official Python/JS SDKs document search/contents/answer/agent/websets — **not** Team Management usage helpers ([exa-py README](https://github.com/exa-labs/exa-py/blob/master/README.md), [exa-js README](https://github.com/exa-labs/exa-js/blob/master/README.md)).

### 2. Websets “Get Team Info” — cheap authenticated probe

```http
GET https://api.exa.ai/websets/v0/teams/me
x-api-key: YOUR-EXA-API-KEY
```

([Get Team Info](https://docs.exa.ai/websets/api/teams/get-team-info); also in [exa-spec.json](https://docs.exa.ai/exa-spec.json) as `GET /v0/teams/me` under server `https://api.exa.ai` — **code samples use the `/websets` prefix**.)

**Returns:**

- `object: "team"`, `id`, `name`
- `concurrency.active` / `concurrency.queued`
- `limits.maxConcurrent` / `limits.maxQueued` (`null` = unlimited)

**Useful for:** validating the key *only if* the key can access Websets.  
**Not useful for:** credit balance, search/contents QPS remaining, or billing plan. Docs frame it as Websets concurrency monitoring ([Get Team Info](https://docs.exa.ai/websets/api/teams/get-team-info)).

**Saku note (2026-07-11):** A search-capable API key can return **401** on `/websets/v0/teams/me` while `/search` still works. Saku’s `status` therefore does **not** live-probe Exa — it only reports whether an API-key Credential is stored for the configured Web Backend.

### 3. Per-request `costDollars` on Search / Contents (and Agent `usage`)

Successful `/search` and `/contents` responses include `costDollars.total` (and optional breakdowns) — **estimated cost of that request**, explicitly **not** an invoice record; “Billing is computed from usage counters rather than this response object” ([exa-spec.json](https://docs.exa.ai/exa-spec.json) `CostDollarsOutput`; [Search](https://docs.exa.ai/reference/search-api-guide-for-coding-agents) / [Contents](https://docs.exa.ai/reference/contents-api-guide-for-coding-agents) response tables).

Agent runs expose per-run `usage` / `costDollars` in the SDK ([exa-js README](https://github.com/exa-labs/exa-js/blob/master/README.md)). Irrelevant to Saku’s current `/search`+`/contents` Web Backend unless Agent is adopted.

**Not suitable for `status`:** requires a billable call; does not report remaining credits.

### 4. Error signals (reactive, not a status API)

| Signal | Meaning | Source |
| --- | --- | --- |
| HTTP `401` | Invalid or missing API key | [Search](https://docs.exa.ai/reference/search-api-guide-for-coding-agents) / [Contents](https://docs.exa.ai/reference/contents-api-guide-for-coding-agents) error tables |
| HTTP `429` | Rate limit exceeded | Same |
| HTTP `402` + `tag: "NO_MORE_CREDITS"` | Credits exhausted | [Nevermined](https://docs.exa.ai/integrations/nevermined) (“When the key runs out”) |
| Monitor `failReason: "insufficient_credits"` | Monitor run failed for credits | [exa-spec.json](https://docs.exa.ai/exa-spec.json) (Monitor run schema) |
| Websets cancel reason `out_of_credits` | Webset search canceled | [exa-spec.json](https://docs.exa.ai/exa-spec.json) / Websets OpenAPI enums |

Search/Contents **guide** error tables list `400/401/422/429` (and `500` for search) but **omit `402`**; the Nevermined page is the primary doc for the credits-exhausted body shape.

---

## Rate limits

| Fact | Source |
| --- | --- |
| Defaults: `/search` **10 QPS**, `/contents` **100 QPS**, `/answer` **10 QPS**, legacy `/research/v1` **15 concurrent** | [Rate Limits](https://docs.exa.ai/reference/rate-limits) |
| Higher limits → Enterprise / `sales@exa.ai` | Same |
| Per-key `rateLimit` configurable via Team Management (documented as requests **per second** in OpenAPI; some prose says “per minute” — **prefer OpenAPI**) | [Create](https://docs.exa.ai/reference/team-management/create-api-key) / [Update](https://docs.exa.ai/reference/team-management/update-api-key) / [team-management-spec.yaml](https://docs.exa.ai/team-management-spec.yaml) |
| Credit balance does **not** raise rate limits | [Billing](https://docs.exa.ai/reference/billing) |

**Remaining QPS / quota in responses:** **Not documented.** Public OpenAPI response headers enumerate only **`X-Request-Id`** ([exa-spec.json](https://docs.exa.ai/exa-spec.json) scan). No `X-RateLimit-*`, `Retry-After`, or remaining-quota headers in the published spec.

---

## Billing / “plan” model (mostly dashboard)

| Fact | API? | Source |
| --- | --- | --- |
| Pay-as-you-go **credits**; charged per usage at [exa.ai/pricing](https://exa.ai/pricing?tab=api) | N/A (pricing page) | [Billing](https://docs.exa.ai/reference/billing), [Pricing](https://exa.ai/pricing?tab=api) |
| **Remaining balance** visible on dashboard Billing page | **Dashboard only** — no balance endpoint in public or team-management OpenAPI | [Billing](https://docs.exa.ai/reference/billing); specs: [exa-spec.json](https://docs.exa.ai/exa-spec.json), [team-management-spec.yaml](https://docs.exa.ai/team-management-spec.yaml) |
| Out of credits → API requests blocked until top-up / auto-recharge | Behavior described; no “get balance” API | [Billing](https://docs.exa.ai/reference/billing) |
| Free: `$10` onboarding credits; `$7`/month if payment method on file (expires monthly) | Dashboard / billing policy | [Billing](https://docs.exa.ai/reference/billing) |
| Auto-recharge, invoices, Stripe | Dashboard | [Billing](https://docs.exa.ai/reference/billing) |
| Teams share usage limits/features; top-up per team in Billing | Dashboard | [Managing Your Team](https://docs.exa.ai/reference/setting-up-team) |
| Marketing “Free Tier / 20k requests/month” | Pricing page (not an API field) | [Pricing](https://exa.ai/pricing?tab=api) |

There is **no** documented “plan name / plan usage remaining” endpoint analogous to Codex Plan Usage.

Dashboard UI also exposes **Usage**, **Billing**, **API Keys**, **Team Settings** (nav observed on [dashboard.exa.ai/api-keys](https://dashboard.exa.ai/api-keys)); those pages are not mirrored as a single “account status” REST resource in the published specs.

---

## Cheap probes: what validates a key?

| Probe | Documented? | Validates key? | Returns plan/credits? |
| --- | --- | --- | --- |
| `GET …/websets/v0/teams/me` | Yes | Yes (auth required) | No — team id/name + Websets concurrency |
| `GET https://admin-api.exa.ai/team-management/api-keys` | Yes | Yes (service key) | No balance; yes `budgetCents` / `isOverBudget` / `rateLimit` per key |
| `GET …/api-keys/{id}/usage` | Yes | Yes | Historical **spend**, not remaining credits |
| `GET /health` or `/account` | **Not in** [exa-spec.json](https://docs.exa.ai/exa-spec.json) (40 paths; none are health/account/balance) | — | — |
| Invalid path / empty search as probe | Not documented as a status mechanism | Would only show 401 vs other errors if tried | No plan info |
| Relying on `costDollars` from a real search | Documented field | Side effect of a paid call | Per-request estimate only |

---

## Explicitly NOT available via documented API

These are **not** exposed by any endpoint in the published public or team-management OpenAPI / guides reviewed:

1. **Remaining credit / dollar balance** (dashboard Billing only — [Billing](https://docs.exa.ai/reference/billing)).
2. **Plan name / free-tier remaining requests** as a live API field.
3. **Remaining rate-limit tokens** or `X-RateLimit-*` headers ([exa-spec.json](https://docs.exa.ai/exa-spec.json) headers = `X-Request-Id` only; [Rate Limits](https://docs.exa.ai/reference/rate-limits) states defaults only).
4. **Account / user profile** endpoint (no `/account` / `/me` outside Websets team concurrency).
5. **Invoice list / auto-recharge config** via API (dashboard — [Billing](https://docs.exa.ai/reference/billing)).
6. First-party SDK helpers for Team Management usage ([exa-py](https://github.com/exa-labs/exa-py), [exa-js](https://github.com/exa-labs/exa-js) READMEs).

---

## Useful for `saku status` — recommendation

| Field / behavior | Worth showing? | Confidence | Notes |
| --- | --- | --- | --- |
| Key valid / invalid (`401` vs success on `GET /websets/v0/teams/me` or list keys) | **Yes** (lightweight “Exa: ok”) | **High** | Documented auth; cheap GET; no search cost. Websets path is slightly odd for a search-only bot but is the only documented non-billable team probe on `api.exa.ai`. |
| Team `name` / `id` from `/websets/v0/teams/me` | Optional | **High** that fields exist; **Medium** usefulness | Confirms which Exa team the key belongs to. |
| Websets `concurrency` / `limits` | **No** (unless Saku uses Websets) | **High** | Not related to `/search`/`/contents` QPS. |
| `total_cost_usd` (+ optional breakdown) for last 30 days via Admin usage API | **Yes, if** service-key auth works with Saku’s stored key **and** key UUID is known/stored | **Medium** | Closest analogue to “usage so far”; **not** “remaining.” Needs `admin-api` + UUID. |
| Per-key `budgetCents` / `isOverBudget` / `rateLimit` from list/get key | **Maybe** (`isOverBudget` is actionable) | **Medium** | Only if Admin API works with the credential; budget is optional and may be null. |
| Remaining credits / “plan left” | **Cannot** (API) | **High** that it’s unavailable | Point users to [dashboard Billing](https://dashboard.exa.ai) / document dashboard-only. |
| Default QPS as static text (10 search / 100 contents) | Optional footnote | **High** for defaults | Not live remaining; from [Rate Limits](https://docs.exa.ai/reference/rate-limits). |
| Last-request `costDollars` | **No** for status | **High** | Wrong shape; costs money to refresh. |
| Infer status from `402 NO_MORE_CREDITS` on tool failures | Useful as **runtime** messaging, not proactive status | **High** for error shape ([Nevermined](https://docs.exa.ai/integrations/nevermined)) | |

### Practical recommendation for Saku

1. **Do not expect a Codex-like “Exa Plan Usage” bar** — Exa does not publish remaining credits over API.
2. **Best proactive status (if implementing):**  
   - Probe `GET https://api.exa.ai/websets/v0/teams/me` → **Exa: connected** (+ team name).  
   - Optionally, if Admin API accepts the same key and you persist the key UUID at `saku login exa`: show **30-day spend** (`total_cost_usd`) and **`isOverBudget`**.  
3. **Confidence overall:** **High** on “no remaining-balance API”; **Medium** on wiring Admin usage into Discord status without a live key experiment and UUID storage; **High** that per-request `costDollars` and Websets concurrency are poor fits for the Codex-usage analogy.

---

## Source index

| Resource | URL |
| --- | --- |
| Docs index (llms.txt) | https://exa.ai/docs/llms.txt |
| Billing | https://docs.exa.ai/reference/billing |
| Rate Limits | https://docs.exa.ai/reference/rate-limits |
| Managing Your Team | https://docs.exa.ai/reference/setting-up-team |
| Get API Key Usage | https://docs.exa.ai/reference/team-management/get-api-key-usage |
| List / Get / Create / Update API Key | https://docs.exa.ai/reference/team-management/list-api-keys · get-api-key · create-api-key · update-api-key |
| Get Team Info (Websets) | https://docs.exa.ai/websets/api/teams/get-team-info |
| Search / Contents coding-agent refs | https://docs.exa.ai/reference/search-api-guide-for-coding-agents · contents-api-guide-for-coding-agents |
| Nevermined (402 / NO_MORE_CREDITS) | https://docs.exa.ai/integrations/nevermined |
| OpenAPI hub | https://docs.exa.ai/reference/openapi-spec |
| Public OpenAPI JSON | https://docs.exa.ai/exa-spec.json |
| Team Management OpenAPI YAML | https://docs.exa.ai/team-management-spec.yaml |
| Pricing | https://exa.ai/pricing?tab=api |
| Dashboard API Keys | https://dashboard.exa.ai/api-keys |
| exa-py / exa-js | https://github.com/exa-labs/exa-py · https://github.com/exa-labs/exa-js |
