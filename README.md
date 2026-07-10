<p align="center">
  <img src="assets/banner.jpg" alt="Cherry blossoms" width="100%" />
</p>

<div align="center">

# saku

咲く — to bloom

</div>

> [!WARNING]
> Early days. No warranty. Things will break.

## Get running

```bash
cargo install --git https://github.com/jameblai/saku
```

## Setup

1. Create a bot in the [Discord Developer Portal](https://discord.com/developers/applications)
2. Enable **Message Content Intent**
3. Invite it to a server — it needs to read/send messages, create threads, and add reactions
4. Run `saku` — paste the bot token and your Discord user id, then finish Codex login
5. Optional: `saku login exa` for web search/extract

## What it is

A coding agent you talk to in Discord threads.

Invite it to a server, @mention the bot, it opens a thread, and you get work done.

I like [Pi](https://github.com/earendil-works/pi) at my desk — focused coding sessions, minimal and simple. On the go, or for background work, I want to chat with an agent that has its own *nix environment: coding and other tasks, without sitting in a terminal. Pi isn’t packaged for that “personal assistant in my pocket” workflow.

[Hermes](https://github.com/NousResearch/hermes-agent) was closer to that shape, but too heavy for what I needed. I was mostly using it as a Discord coding agent, and it shipped skills, connectors, and other platforms I didn’t need — Python, a long install, and about 500k lines of Python (excluding tests and web UI) against ~9k lines of Rust here.

Saku is what I actually wanted: one binary, Discord as the UI, enough tools to get the job done.

Discord’s threading is ideal for agents — work and context stay in one place. Better than Telegram (buggy), Slack (too corporate), and the rest that don’t really do threads.

### What you get

- `read` / `edit` / `write` / `bash` — enough to work a codebase
- Web search & extract via Exa
- Basic memory across sessions
- Fast file search ([fff](https://github.com/dmtrKovalenko/fff))
- A minimal system prompt
- No permission popups

## Contributing

This is mostly a personal project. Small, focused PRs are welcome — big drive-by refactors or thousand-line dumps probably aren't. If you're unsure whether something fits, open an issue first.

## License

MIT
