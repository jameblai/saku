#!/usr/bin/env bash
set -euo pipefail

repo="jameblai/saku"
channel="stable"
assume_yes=false

usage() {
  echo "Usage: install.sh [--channel stable|nightly] [--yes]"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --channel)
      [[ $# -ge 2 ]] || { usage >&2; exit 2; }
      channel="$2"
      shift 2
      ;;
    --yes) assume_yes=true; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done
[[ "$channel" == "stable" || "$channel" == "nightly" ]] || {
  echo "Channel must be stable or nightly" >&2
  exit 2
}

[[ "$(uname -s)" == "Linux" ]] || { echo "Saku supports Linux only." >&2; exit 1; }
case "$(uname -m)" in
  x86_64) arch=x86_64 ;;
  aarch64|arm64) arch=aarch64 ;;
  *) echo "Unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

for command in curl tar sha256sum; do
  command -v "$command" >/dev/null || { echo "Required command not found: $command" >&2; exit 1; }
done

if [[ "$channel" == "stable" ]]; then
  tag=$(curl -fsSL "https://api.github.com/repos/$repo/releases/latest" | sed -n 's/.*"tag_name":[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)
else
  tag=$(curl -fsSL "https://api.github.com/repos/$repo/releases?per_page=100" | tr '{' '\n' | sed -n 's/.*"tag_name":[[:space:]]*"\([^"]*-nightly\.[^"]*\)".*/\1/p' | head -1)
fi
[[ -n "$tag" ]] || { echo "No $channel Release found." >&2; exit 1; }

asset="saku-linux-$arch.tar.gz"
base="https://github.com/$repo/releases/download/$tag"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fL --retry 3 "$base/$asset" -o "$tmp/$asset"
curl -fL --retry 3 "$base/SHA256SUMS" -o "$tmp/SHA256SUMS"
(cd "$tmp" && grep "  $asset\$" SHA256SUMS | sha256sum --check --status) || {
  echo "Checksum verification failed for $asset" >&2
  exit 1
}
tar -xzf "$tmp/$asset" -C "$tmp" saku
install -d "$HOME/.local/bin"
install -m 0755 "$tmp/saku" "$HOME/.local/bin/saku"

write_release_channel() {
  local config="$HOME/.saku/config.toml"
  mkdir -p "$(dirname "$config")"
  if [[ -f "$config" ]]; then
    sed -i '/^[[:space:]]*release_channel[[:space:]]*=/d' "$config"
  fi
  printf '\nrelease_channel = "%s"\n' "$channel" >> "$config"
  chmod 600 "$config"
}

ensure_local_bin_on_path() {
  case ":$PATH:" in
    *":$HOME/.local/bin:"*) ;;
    *)
      local rc="$HOME/.profile"
      [[ "${SHELL:-}" == */zsh ]] && rc="$HOME/.zshrc"
      printf '\nexport PATH="$HOME/.local/bin:$PATH"\n' >> "$rc"
      echo "Added ~/.local/bin to PATH in $rc"
      ;;
  esac
}

write_release_channel
ensure_local_bin_on_path

interactive=false
if [[ -t 1 && -e /dev/tty && "$assume_yes" == false ]]; then interactive=true; fi
if [[ "$interactive" == false ]]; then
  echo "Installed Saku $tag to ~/.local/bin/saku."
  echo "Next: run ~/.local/bin/saku setup, then configure the user service (re-run this installer interactively)."
  exit 0
fi

"$HOME/.local/bin/saku" setup < /dev/tty
unit_dir="$HOME/.config/systemd/user"
mkdir -p "$unit_dir"
cat > "$unit_dir/saku.service" <<EOF
[Unit]
Description=Saku Discord coding agent

[Service]
ExecStart=$HOME/.local/bin/saku
Restart=on-failure

[Install]
WantedBy=default.target
EOF
loginctl enable-linger "$USER"
systemctl --user daemon-reload
systemctl --user enable --now saku.service
echo "Installed Saku $tag and started saku.service."
