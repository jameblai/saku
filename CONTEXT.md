# Saku

A Discord-hosted coding agent: authorised users @mention the bot, and it runs a tool-using agent loop against a workspace.

## Language

**Saku**:
The Discord coding agent product (bot + harness).
_Avoid_: pi, Discord bot (alone), coding agent (alone)

**Harness**:
The agent runtime that runs the LLM ↔ tool loop (prompt, tool calls, results, repeat until done).
_Avoid_: agent framework, orchestrator, runtime (alone)

**Data Dir**:
Saku’s private state directory at `~/.saku` (config, credentials, Session Store, Memory, FFF DBs, etc.).
_Avoid_: workspace (different), home (may coincide but is not the term)

**Workspace**:
The configured filesystem confine root (from `config.toml`, defaulting to the bot user’s home directory). All Authorised Users share it. File tools and `cd` cannot resolve paths outside this root after canonicalization.
_Avoid_: project, repo, cwd, Data Dir

**Working Directory**:
The current directory for a Session’s tools (`bash`, relative paths, `ls`, etc.). Starts at the Workspace root; changed only by the `cd` Tool, and only to directories inside the Workspace.
_Avoid_: Workspace (the confine root), cwd (as the domain term)

**Authorised User**:
A Discord account whose snowflake id is on the allowlist; only they may invoke the harness via @mention.
_Avoid_: admin, operator, owner (unless those mean something else later)

**Session**:
One agent conversation bound to a single Discord thread. A channel @mention creates (or attaches to) a thread and starts/continues that Session there. Further messages from Authorised Users in that thread continue the Session without requiring another @mention. Each Session has its own Working Directory and Read Snapshots.
_Avoid_: conversation, chat, context (as the domain term for this binding)

**Memory**:
A file at `~/.saku/MEMORY.md` holding durable facts the agent should retain across Sessions — only explicitly requested memories or clearly important stable preferences/facts (e.g. rough location, tech stack, response style). Hard cap: 2,200 characters. Contents are included in every Run. Written only via the `memory` Tool (full replace; empty content clears). File tools cannot access this path. Not a scrapbook of trivia.
_Avoid_: notes, journal, context file, AGENTS.md

**Run**:
One invocation of the Harness for a user message (or queued follow-up) until the agent stops: LLM turns and tool calls for that request. Exposed to adapters as a `RunHandle` streaming rich `RunEvent`s (deltas, tool lifecycle, queue, finished/error/aborted).
_Avoid_: job, task, turn (turn = one LLM call inside a Run)


**Subagent**:
An isolated inner agent loop spawned by the parent Run via the `subagent` Tool. Gets its own mini-transcript (task string only — no parent history), inherits Project Context / Memory / Skills, and returns a summary to the parent. Modes: `explore` (read-only tools, default) or `edit` (full tools minus nested `subagent`). Depth 1 only; up to 4 parallel children per tool batch when all are `explore`.
_Avoid_: Session, Background Process, delegate (as the tool name)


**Provider**:
A pluggable LLM backend identity (e.g. Codex subscription, a future Anthropic subscription, or an API-key vendor). Credentials and request auth are resolved per Provider. At the code boundary, a Provider streams completion events from a harness `Request` (transcript + tools + model + Effort).
_Avoid_: model (a Provider exposes one or more models), backend (alone), LLM, Web Backend


**Web Backend**:
A pluggable public-web service that implements search and/or extract for the Harness (Exa first; others later). Not an LLM Provider.
_Avoid_: Provider, search provider, web provider


**Credential**:
Stored auth for one Provider or Web Backend — OAuth tokens or an API key — used to authorize that identity’s requests and refreshed when expired. Persisted pi-style in `~/.saku/auth.json` (id keys; `oauth` / `api_key` tagged entries; file mode `0600`; locked refresh).
_Avoid_: token, secret, api key (as the umbrella term)


**Login**:
A host CLI flow (`saku login <id>`) that obtains and stores a Credential for a Provider or Web Backend (Codex uses device-code OAuth; API-key identities accept a pasted key). Not performed inside Discord. Implemented in `saku-cli`, writing via the harness Credential store. Also run as the final step of Setup for the default Provider.
_Avoid_: /login, sign-in (as product UI inside Discord)

**Install**:
Host-level delivery of the `saku` binary onto the machine (e.g. `curl … | bash` or `saku update`). Does not write the Data Dir or perform Login. A typical interactive Install chains into **Setup**, then installs a **Host Service** unit; turning it on is a separate operator choice via `saku service enable`. Non-interactive Install delivers only the binary.
_Avoid_: Setup, onboarding, deploy (as the product term)

**Update**:
Host CLI flow (`saku update`) that replaces the installed `saku` binary from the chosen Release Channel and restarts the **Host Service** if it is active. Unit file changes require `saku service install`, not Update.
_Avoid_: upgrade (alone), reinstall

**Host Service**:
The user-level systemd unit that keeps the `saku` binary running across logout; managed via `saku service` commands (`install`, `uninstall`, `enable`, `disable`). `install` writes the unit and enables linger; `enable` turns on persistence and starts the service now; `disable` stops it and turns off persistence. Bare `saku service` reports status.
_Avoid_: service (alone), daemon, Background Process

**Release Channel**:
Which delivery line Install or Update follows — **stable** (semver-tagged prebuilt releases, GitHub `latest`), **nightly** (automated prebuilt prereleases from `main`), or **git** (clone and compile `main` locally). Persisted in `config.toml` (default `stable`); `saku update` follows it. Changed only by editing config, not a CLI flag.
_Avoid_: branch, track, version pin (as the product term)

**Setup**:
A host CLI flow (`saku setup`) that collects Discord bot token and Authorised User id(s) (one prompt; comma- or space-separated snowflakes), writes those required fields into `config.toml` in the Data Dir (optional keys left to defaults; re-runs preserve existing optional keys), then runs Codex Login. `saku` auto-enters Setup when config is missing or cannot be loaded (IO, parse, or missing required fields), with best-effort prefill; if config is present but the Codex Credential is missing, it auto-runs Login only. Re-runs prefill existing config values and always finish with Login. When Setup/Login was entered automatically from bare `saku`, success continues into the bot; explicit `saku setup` exits after success.
_Avoid_: onboarding, install, init, first-run wizard (as product terms)


**Tool**:
A named capability the model may call during a Run. Core set: `bash`, `read`, `edit`, `write`, `find`, `grep`, `ls`, `cd`, `memory`, `bg_start`, `bg_list`, `bg_logs`, `bg_stop`, `session_search`, `subagent`. Web set: `web_search`, `web_extract` — registered only when a Credential exists for the configured Web Backend. `find` and `grep` are backed by FFF (pi-fff semantics); `ls` is a thin directory listing; `cd` changes the Session Working Directory within the Workspace; `memory` full-replaces Memory; Background Process Tools manage Session-scoped long-running processes; `session_search` queries the Session Search FTS index; `subagent` runs an isolated inner agent loop; web Tools call that Web Backend. Registered on the Harness via a dynamic schema + `execute` interface (JSON args in, content parts out).
_Avoid_: function, action, skill


**Skill**:
An Agent Skills standard capability package (`SKILL.md` plus optional scripts and references) that Saku discovers and loads on demand. Each Run injects available skill names and descriptions into the System Prompt; the model loads full instructions via `read`, or the user forces a skill with `$skill-name` in their message. Global skills live under `~/.agents/skills/`; project skills under `.agents/skills/` in the Session Working Directory and its ancestors up to the Workspace root.
_Avoid_: Tool, extension, prompt template, MCP


**Background Process**:
A Session-scoped OS process started by the agent via `bg_start` that outlives the Run that started it (e.g. an HTTP server). Starts in the Session Working Directory at call time (frozen for that process). Soft cap of 5 *running* per Session; output captured in an in-memory ~1 MiB ring buffer (drop oldest). Listed/logged/stopped via `bg_list` / `bg_logs` / `bg_stop` Tools and the `saku bg` Bot Command family. Not killed by Run end or `saku stop` (those remain Run-only). Authorised Users cannot *start* Background Processes via Bot Command — only inspect and stop them. In-memory for the bot process lifetime (not replayed from the Session Store).
_Avoid_: job, daemon, service (as the domain term), bash (foreground tool)


**Vision**:
Image understanding via the Provider: images attached to the triggering Discord message, and image files returned from `read`, are resized/compressed then sent as multimodal content on that turn. Older thread images are not auto-scraped.
_Avoid_: OCR, screenshot tool (unless added later)

**Run Queue**:
At most one active Run per Session. Additional messages in that Session wait (hourglass reaction on the waiting message, no text reply). Separate Sessions may Run in parallel. `stop` aborts the active Run and drops the rest of that Session’s queue (hourglasses cleared).
_Avoid_: global lock, job queue (as the product term)

**Read Snapshot**:
Record of a Workspace file’s identity (path + content hash/mtime) taken when `read` succeeds; lives for the whole Session. `edit` may only proceed if a matching snapshot exists and the file is unchanged. `write` may create a path that does not exist; if the path already exists, the same snapshot rule as `edit` applies.
_Avoid_: lock, etag (as the domain term)

**Session Store**:
On-disk persistence under the Data Dir of a Session’s transcript, Read Snapshots, Working Directory, and model/Effort, keyed by Discord thread id. Format: append-only JSONL per thread (`~/.saku/sessions/<thread_id>.jsonl`), pi-inspired entry types but linear (no branch tree). Replayed to rebuild Session state after restart.
_Avoid_: database, cache, memory (for this durable store), JSONC


**Session Search**:
Cross-Session full-text search over past conversation text, backed by a SQLite FTS index under the Data Dir. The agent invokes it via a `session_search` Tool; returns ranked snippets (thread id, date, role, excerpt) from user/assistant text and compaction summaries across all Sessions.
_Avoid_: Memory, find/grep (Workspace files), session replay


**Command Prefix**:
A configurable string from `~/.saku/config.toml` that introduces Bot Commands (default `saku` → `saku stop`, `saku model gpt-5.5`). Parsed as the prefix token plus whitespace before the subcommand.
_Avoid_: slash command (unless Discord slash commands are added later)


**Bot Command**:
A user message in a Session thread that starts with the Command Prefix and is handled by Saku rather than sent to the Harness as a normal prompt. v1: `stop`, `help`, `steer <message>`, `model`, `effort`, `status`, `goal`, `goal <condition>`, `bg`, `bg logs <pid>`, `bg stop <pid>`, `bg stop all`.
_Avoid_: slash command, reaction cancel (not used for cancel in v1)

**Plan Usage**:
The Codex subscription’s rolling rate-limit windows — **5h** and **weekly** remaining allowance, each with a reset time — fetched live for `status`. Not a calendar-day quota.
_Avoid_: daily usage, quota (alone), rate limit % (alone)

**Reset Credit**:
One banked Codex rate-limit reset the user can spend later. `status` is read-only: it shows available count and soonest expiry; redeeming is not a Bot Command (yet).
_Avoid_: reset bank token, usage voucher

**Steer**:
A Bot Command that injects a mid-Run user directive into the active Session; applied after the current tool batch finishes (does not start a second parallel Run in that Session).
_Avoid_: interrupt, follow-up (follow-up is a normal post-Run message)


**Goal**:
A Session-scoped completion condition set via `saku goal <condition>`. While active, the Harness chains Runs until a Goal Evaluator judges the condition met or `max_runs` (20, fixed in v1) is exhausted. Persisted in the Session Store and resumes after bot restart. Cleared on achievement or `saku stop` (which also aborts the active Run and clears the queue). Replaced when a new `saku goal` is issued.
_Avoid_: Run, cron, steer, task


**Goal Evaluator**:
A short Harness invocation after each working Run on `gpt-5.4-mini` at low Effort. Reads the full Session transcript and returns a structured `goal_check` verdict (`met` + `reason`). Drives whether the outer Goal loop continues.
_Avoid_: compaction, steer, subagent


**Progress Message**:
A Discord reply to the user’s triggering message that lists Tool calls as they happen (emoji + tool name + short args preview). Edited in place during the Run; left in the thread afterward.
_Avoid_: status embed, log dump

**Answer Message**:
A separate Discord reply to the same user message containing the Run’s final assistant text (chunked if needed), posted after tools finish.
_Avoid_: Progress Message (different message)

**Typing Indicator**:
Discord’s ephemeral “is typing…” signal in the Session thread for the duration of an active Run (from Run start / `Dequeued` until the Run terminates). Not shown while queued; not used for Bot Commands.
_Avoid_: presence, status, activity

**Compaction**:
A pi-style reduction of a Session transcript: older turns are summarized into a compact entry so the Harness stays within the model context window while recent messages and Memory remain available.
_Avoid_: truncate, summarize (alone), prune

**System Prompt**:
The fixed Harness instructions sent each Run (identity, Workspace/cwd, tool norms), always combined with injected Memory and **Project Context**. Wording is minimal and iterated in code.
_Avoid_: persona doc, AGENTS.md (as the domain term — use Project Context)

**Project Context**:
Repository and personal norms from discovered `AGENTS.md` files, fully injected into the System Prompt each Run. Loaded from `~/.agents/AGENTS.md` (if present) plus every `AGENTS.md` in the Session Working Directory and its ancestors up to the Workspace root, ordered global first then root→cwd.
_Avoid_: Memory, Skill, context file (generic)

**Effort**:
The reasoning/thinking level sent to the current model (pi’s thinking levels: e.g. `minimal`, `low`, `medium`, `high`, `xhigh`, and `max` where the model supports it). Available values depend on the selected model. Chosen per Session; new Sessions take model/Effort defaults from `config.toml`.
_Avoid_: thinking (as the user-facing command name), temperature

