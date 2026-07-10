# ADR 0015: Discord progress and reply UX

## Status

Accepted

## Context

Saku must show live agent work in Discord without spam, and make completion obvious. The user wants a pattern used on a similar personal bot: hourglass while queued, a growing tools list as a reply, a separate final answer reply, and a checkmark on the original user message when done. Bot Commands use the same reply-to-user pattern.

## Decision

### Reactions

- **Queued / waiting** (Run Queue): hourglass reaction on the waiting user message; remove when the Run for that message starts or is cancelled.
- **Completed successfully**: checkmark reaction on the original user message that triggered the Run (or Bot Command).
- No textual “queued” reply.

### Replies

- All bot output for a user message (Progress Message, Answer Message, Bot Command responses) is a **reply** to that user message.

### Progress Message (tools)

- One reply, edited as Tools execute, lines like:

```text
Tools:
🔧 edit: `src/pages/og.png.ts`
💻 bash: `pnpm lint`
📖 read: `src/pages/og.png.ts`
🔍 find: `og.png`
🔎 grep: `color-violet`
📁 ls: `src/pages`
📂 cd: `jamesblair.nz`
✍️ write: `src/new-file.ts`
```

Suggested emoji map (v1 Tools only — no separate vision tool; images arrive on the user turn / via `read`):

| Tool | Emoji |
|------|--------|
| `bash` | 💻 |
| `read` | 📖 |
| `edit` | 🔧 |
| `write` | ✍️ |
| `find` | 🔍 |
| `grep` | 🔎 |
| `ls` | 📁 |
| `cd` | 📂 |

- Args preview is an **inline code span** (not ASCII quotes): collapse whitespace, truncate for length, then wrap with a Discord backtick fence one longer than the longest backtick run inside the arg (pad with spaces when the content starts or ends with a backtick). This keeps machine strings (`||`, `*`, etc.) from being interpreted as Discord markdown. Answer Messages stay normal markdown.
- Args preview truncated for Discord length; keep the Progress Message under Discord limits (trim oldest tool lines or collapse if needed).
- Leave the Progress Message in the thread after completion (audit trail).

### Answer Message

- After the Run finishes, post a **new** reply to the same user message with the final assistant text (chunk across multiple replies if >2000 chars).
- On `RunError` / `RunAborted`, reply with a short error/aborted note; clear hourglass; **no** checkmark on failure (❌ optional later).

### Bot Commands

- Responses to `help` / `model` / `effort` / `stop` / `steer` are replies to the command message.

### `stop` and the Run Queue

- Abort the active Run; **drop** remaining queued messages in that Session; clear their hourglasses.

## Consequences

- `saku-discord` maps `RunEvent` tool lifecycle → Progress Message edits; `RunFinished` → Answer Message + ✅ on OG message.
- Vision does not appear as its own Tools line unless the model `read`s an image path.
