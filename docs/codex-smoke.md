# Codex smoke test (manual)

Live Codex is not exercised in CI. After implementing ticket #9:

1. Ensure config exists (`saku setup`, or a hand-written `~/.saku/config.toml` with `discord_token` and `authorized_user_ids`; token unused for login).
2. Run `cargo run -p saku -- login codex` (or finish Setup’s Login step).
3. Open the printed verification URL, enter the user code, wait for “credentials saved”.
4. Confirm `~/.saku/auth.json` exists with mode `0600` and a `codex` oauth entry.
5. Optional: write a small harness binary/test that constructs `CodexProvider` with the Credential store and runs one text-only `Session::run` against a temp Workspace (costs quota).

CI continues to use `FakeProvider` only.
