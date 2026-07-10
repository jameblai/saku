# ADR 0021: Prebuilt binary Install and dual-channel Releases

## Status

Accepted

## Context

Saku previously required compiling on the host — too heavy for underpowered machines (e.g. Raspberry Pi) and conflicting with the goal of a portable, lightweight agent. Issue #39 needs host-level **Install** (prebuilt binary + user systemd service), **Update** (`saku update`), and a release pipeline. **Setup** (Discord config + Login) stays separate but chains into interactive Install.

We considered rolling `main` releases only, stable semver only (Starship-style), and t3code’s dual stable+nightly model. Pi iteration wants frequent bleeding-edge builds without forcing the maintainer to tag every change; stable tags remain intentional and low-anxiety.

## Decision

- **Platforms (v1):** prebuilt `linux-x86_64` and `linux-aarch64` only — no `armv7`, no Darwin.
- **Artifacts:** GitHub Releases — per-arch tarball + `SHA256SUMS`. Install script hosted at `raw.githubusercontent.com` (pin to tag for stable); no branded subdomain in v1.
- **Dual Release Channels (t3code-lite):**
  - **Stable:** manual `vX.Y.Z` tag push or `workflow_dispatch`; marked GitHub `latest`; first stable is `v0.1.0`.
  - **Nightly:** cron every **3 hours**, skip if `main` unchanged since last nightly tag; always prerelease, never `latest`. Tag format: `v{base}-nightly.{YYYYMMDD}.{run}` (e.g. `v0.1.0-nightly.20260711.3`).
- **Install:** user-level binary at `~/.local/bin/saku`; `install.sh` default channel **stable**; `--channel nightly` install-time only. Interactive flow: binary → write `release_channel` in config → **Setup** → **Host Service** install (ADR-0022). Non-interactive: binary only + next-step hints.
- **Update:** `saku update` subcommand reads **Release Channel** from `config.toml` (default `stable`); checksum-verify; replace binary; `systemctl --user restart saku` if active. No `--channel` CLI flag — change channel by editing config.
- **Host Service:** see ADR-0022 (`saku service` commands; unit at `~/.config/systemd/user/saku.service`, `Restart=on-failure`).

## Consequences

- `config.toml` gains optional `release_channel` (`stable` | `nightly`); ADR-0012 defaults doc should list it.
- New CI workflow for release builds (tag + nightly cron + dispatch); existing CI unchanged for PRs.
- README **Get running** is operator-only: `install.sh` + **Setup** steps. `## Contributing` is a single link to `CONTRIBUTING.md`.
- Add `CONTRIBUTING.md`: `git clone`, `cargo run -p saku`, `cargo build --release -p saku`, and CI-parity checks (`cargo test --workspace`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`); PR etiquette moved from README.
- Do not document a source-install command anywhere.
- Host service management: ADR-0022.
