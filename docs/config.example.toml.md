# Example Saku config (`~/.saku/config.toml`)

Prefer **`saku setup`** for first-run (prompts for Discord bot token + Authorised User ids, then Codex Login). Bare **`saku`** also auto-enters Setup when config is missing/invalid, or Login when the Codex Credential is missing (ADR-0017).

```toml
discord_token = "YOUR_BOT_TOKEN"
authorized_user_ids = ["YOUR_DISCORD_SNOWFLAKE"]

# Optional overrides:
# command_prefix = "saku"
# workspace = "~"
# data_dir = "~/.saku"
# default_model = "gpt-5.5"
# default_effort = "medium"
# web_backend = "exa"
# release_channel = "stable" # or "nightly"; used by `saku update`
```

Re-auth: `saku login codex` (Provider) or `saku login exa` (Web Backend API key). Start the bot: `saku`.
