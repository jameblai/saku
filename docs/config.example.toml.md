# Example Saku config (`~/.saku/config.toml`)

Prefer **`saku setup`** for first-run (prompts for Discord bot token + Authorised User ids, then Codex Login).

```toml
discord_token = "YOUR_BOT_TOKEN"
authorized_user_ids = ["YOUR_DISCORD_SNOWFLAKE"]

# Optional overrides:
# command_prefix = "saku"
# workspace = "~"
# data_dir = "~/.saku"
# default_model = "gpt-5.5"
# default_effort = "medium"
```

Re-auth only: `saku login codex`. Start the bot: `saku`.
