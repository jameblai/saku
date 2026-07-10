# Contributing

Small, focused pull requests are welcome. For large refactors or uncertain scope, open an issue first.

## Development

Install [Rust stable](https://rustup.rs/); `rust-toolchain.toml` selects the repository toolchain. Then:

```bash
git clone git@github.com:jameblai/saku.git
cd saku
cargo run -p saku
```

For a production-like build:

```bash
cargo build --release -p saku
```

Before opening a pull request, run the same checks as CI:

```bash
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```
