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
A file at `~/.saku/MEMORY.md` holding durable facts the agent should retain across Sessions — only explicitly requested memories or clearly important stable preferences/facts (e.g. rough location, tech stack, response style). Hard cap: 2,200 characters. Contents are included in every Run. File tools may always read/write this exact path even when it lies outside the Workspace; writes over the cap are rejected. Not a scrapbook of trivia.
_Avoid_: notes, journal, context file, AGENTS.md

**Run**:
One invocation of the Harness for a user message (or queued follow-up) until the agent stops: LLM turns and tool calls for that request.
_Avoid_: job, task, turn (turn = one LLM call inside a Run)

**Provider**:
A pluggable LLM backend identity (e.g. Codex subscription, a future Anthropic subscription, or an API-key vendor). Credentials and request auth are resolved per Provider.
_Avoid_: model (a Provider exposes one or more models), backend, LLM

**Credential**:
Stored auth for one Provider — either OAuth tokens (subscription login) or an API key — used to authorize model requests and refreshed when expired.
_Avoid_: token, secret, api key (as the umbrella term)

**Login**:
A host CLI flow (`saku login <provider>`) that obtains and stores a Credential for a Provider (Codex uses device-code OAuth). Not performed inside Discord.
_Avoid_: /login, sign-in (as product UI inside Discord)

**Tool**:
A named capability the model may call during a Run. v1 set: `bash`, `read`, `edit`, `write`, `find`, `grep`, `ls`, `cd`. `find` and `grep` are backed by FFF (pi-fff semantics); `ls` is a thin directory listing; `cd` changes the Session Working Directory within the Workspace.
_Avoid_: function, action, skill

**Vision**:
Image understanding via the Provider: images attached to the triggering Discord message, and image files returned from `read`, are resized/compressed then sent as multimodal content on that turn. Older thread images are not auto-scraped.
_Avoid_: OCR, screenshot tool (unless added later)

**Run Queue**:
At most one active Run per Session. Additional messages in that Session wait (hourglass reaction on the waiting message, no text reply). Separate Sessions may Run in parallel.
_Avoid_: global lock, job queue (as the product term)

**Read Snapshot**:
Record of a Workspace file’s identity (path + content hash/mtime) taken when `read` succeeds; lives for the whole Session. `edit` may only proceed if a matching snapshot exists and the file is unchanged. `write` may create a path that does not exist; if the path already exists, the same snapshot rule as `edit` applies.
_Avoid_: lock, etag (as the domain term)

**Session Store**:
On-disk persistence under the Data Dir of a Session’s transcript, Read Snapshots, and Working Directory, keyed by Discord thread id, so Runs survive bot restarts.
_Avoid_: database, cache, memory (for this durable store)

**Command Prefix**:
A configurable string from `~/.saku/config.toml` that introduces Bot Commands (default `saku` → `saku stop`, `saku model gpt-5.5`). Parsed as the prefix token plus whitespace before the subcommand.
_Avoid_: slash command (unless Discord slash commands are added later)


**Bot Command**:
A user message in a Session thread that starts with the Command Prefix and is handled by Saku rather than sent to the Harness as a normal prompt. v1: `stop`, `help`, `steer <message>`, `model`, `effort`.
_Avoid_: slash command, reaction cancel (not used for cancel in v1)

**Steer**:
A Bot Command that injects a mid-Run user directive into the active Session; applied after the current tool batch finishes (does not start a second parallel Run in that Session).
_Avoid_: interrupt, follow-up (follow-up is a normal post-Run message)

**Compaction**:
A pi-style reduction of a Session transcript: older turns are summarized into a compact entry so the Harness stays within the model context window while recent messages and Memory remain available.
_Avoid_: truncate, summarize (alone), prune

**Effort**:
The reasoning/thinking level sent to the current model (pi’s thinking levels: e.g. `minimal`, `low`, `medium`, `high`, `xhigh`, and `max` where the model supports it). Available values depend on the selected model. Chosen per Session; new Sessions take model/Effort defaults from `config.toml`.
_Avoid_: thinking (as the user-facing command name), temperature

