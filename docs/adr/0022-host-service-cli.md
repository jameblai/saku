# ADR 0022: Host Service CLI

## Status

Accepted

## Context

ADR-0021 interactive **Install** wrote the systemd unit, enabled linger, and ran `systemctl --user enable --now` from `install.sh`. That couples host-service logic to bash, auto-enrolls operators in a running daemon, and overlaps vocabulary with **Background Process** (which explicitly avoids “service” as a domain term). Grilling (#39 follow-on) separated binary delivery from opt-in service enablement and agreed on a Rust-owned CLI surface.

## Decision

- **Host Service** is the glossary term for `~/.config/systemd/user/saku.service` (see `CONTEXT.md`).
- Implement management in **`saku-cli`** as `saku service` subcommands — not in **Setup**, not in bash beyond delegation:
  - `install` — write unit file + `loginctl enable-linger`
  - `uninstall` — stop if active, disable, remove unit, `daemon-reload` (linger left on)
  - `enable` — `systemctl --user enable --now`
  - `disable` — `systemctl --user disable` + stop if active
  - bare `saku service` — status (installed / enabled / active / linger); `--help` for usage
  - No `start` / `stop` subcommands
- **`install.sh` (interactive):** after **Setup**, call `saku service install`, then hint the operator to run `saku service enable` when ready.
- **`saku update`:** binary replacement + restart only if the unit is already active. Unit template drift is reconciled by re-running `saku service install`, not by Update.

## Consequences

- `scripts/install.sh` drops inline systemd/linger/enable logic in favour of calling `saku service install`.
- ADR-0021 interactive Install bullet is superseded for host-service steps; releases/channels/binary paths unchanged.
