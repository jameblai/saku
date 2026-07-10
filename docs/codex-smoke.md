# Codex smoke test (manual)

Live Codex is not exercised in CI. After implementing ticket #9:

1. Ensure `~/.saku/config.toml` exists with at least `discord_token` and `authorized_user_ids` (token unused for login).
2. Run `cargo run -p saku -- login codex`.
3. Open the printed verification URL, enter the user code, wait for “credentials saved”.
4. Confirm `~/.saku/auth.json` exists with mode `0600` and a `codex` oauth entry.
5. Optional: write a small harness binary/test that constructs `CodexProvider` with the Credential store and runs one text-only `Session::run` against a temp Workspace (costs quota).

CI continues to use `FakeProvider` only.
