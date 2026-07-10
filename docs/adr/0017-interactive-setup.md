# ADR 0017: Interactive Setup CLI

## Status

Accepted

## Context

A missing `~/.saku/config.toml` currently hard-fails with a raw IO error. First-run needs Discord bot token, Authorised User snowflake(s), correct Gateway intents, and a Codex Credential — without forcing users to hand-write TOML or discover `saku login` separately. We considered “print a hint and exit” vs auto-entering Setup, and whether Setup should subsume Login or stay separate.

## Decision

- Add **Setup** (`saku setup`): prompt for bot token (hidden input) and Authorised User id(s) (comma/space-separated, digits-only), write required fields to `config.toml` (`0600`), preserve any existing optional keys on re-run, then run Codex **Login**.
- Bare `saku` **auto-enters Setup** when config is missing or cannot be loaded (IO, parse, or missing required fields), with best-effort prefill. If config loads but the Codex Credential is missing, **auto-run Login only**.
- After auto-entered Setup/Login succeeds, **continue into the bot**. Explicit `saku setup` **exits** after success.
- If config write succeeds but Login fails, **keep the config**; the next `saku` recovers via Login-only.
- Re-runs **prefill** existing required values and **always** finish with Login. `saku login` remains for re-auth without full Setup.
- Setup help text: Developer Portal one-liner; intents checklist (`GUILDS`, `GUILD_MESSAGES`, `MESSAGE_CONTENT` privileged, `GUILD_MESSAGE_REACTIONS`); one-liner for Developer Mode → Copy User ID. Do not prompt for optional config (workspace, prefix, model, effort).
- CLI crates: **`clap`** for subcommands/`--config`; **`rpassword`** for hidden token input; std for visible prompts. No `dialoguer`/`inquire`.

## Consequences

- Example config docs should describe Setup (and auto-entry) rather than “create TOML by hand, then login.”
- ADR-0013’s suggested `clap` dependency becomes real when this ships.
