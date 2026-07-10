<p align="center">
  <img src="assets/banner.jpg" alt="Cherry blossoms" width="100%" />
</p>

<div align="center">

# saku

咲く — to bloom

*Early days. No warranty. Things will break.*

</div>

## Get running

```bash
cargo install --git https://github.com/jameblai/saku
```

A curl installer will come once releases are ready.

## Setup

1. Create a bot in the [Discord Developer Portal](https://discord.com/developers/applications)
2. Enable **Message Content Intent**
3. Invite it to a server — it needs to read/send messages, create threads, and add reactions (or just give it Administrator)
4. Run `saku` — paste the bot token and your Discord user id, then finish Codex login
5. Optional: `saku login exa` for web search/extract

## What it is

A coding agent you talk to in Discord threads.

Invite it to a server, @mention the bot, it opens a thread, and you get work done.

I wanted something lighter than [Hermes](https://github.com/NousResearch/hermes-agent). I was mostly using it as a Discord coding agent, but it shipped skills, connectors, and other platforms I didn’t need — Python, a long install, orders of magnitude more code than this. [pi](https://github.com/earendil-works/pi) pointed at the minimal agent loop. Saku is the cut-down version I actually wanted: one binary, Discord as the UI, enough tools to get the job done.

Discord’s threading is ideal for agents — work and context stay in one place. Better than Telegram (buggy), Slack (too corporate), and the rest that don’t really do threads.

### What you get

- `read` / `edit` / `write` / `bash` — enough to work a codebase
- Web search & extract via Exa
- Basic memory across sessions
- Fast file search (fff)
- A minimal system prompt
- No permission popups

## License

MIT
