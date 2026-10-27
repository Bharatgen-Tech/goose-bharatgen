#!/usr/bin/env bash
set -euo pipefail

REPO="Bharatgen-Tech/goose-bharatgen"
CONFIG_DIR="${HOME}/.config/bg-code"
ENV_FILE="${CONFIG_DIR}/env"

url=""
key=""
model=""

usage() {
  cat <<EOF
Usage: install.sh [--url <gateway-url>] [--key <virtual-key>] [--model <name>] [--desktop]

Prompts for anything not passed. Configures bg-code either through the
Manch LiteLLM gateway or directly with an OpenRouter API key.
EOF
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --url) url="$2"; shift 2 ;;
    --key) key="$2"; shift 2 ;;
    --model) model="$2"; shift 2 ;;
    --desktop) desktop=1; shift ;;
    *) echo "Unknown option: $1" >&2; usage ;;
  esac
done

os="$(uname -s | tr '[:upper:]' '[:lower:]')"
case "$os" in
  mingw*|msys*|cygwin*) os="windows" ;;
esac
[[ "$os" == "darwin" || "$os" == "linux" || "$os" == "windows" ]] || { echo "Unsupported OS: $os" >&2; exit 1; }

latest_release() {
  curl -fsSL "https://api.github.com/repos/${REPO}/releases/tags/stable" | \
    grep -oE '"browser_download_url": *"[^"]+"' | grep -oE 'https[^"]+' | grep "$1"
}

ask_key() {
  if [[ -z "$key" ]]; then
    read -r -s -p "OpenRouter API key (from openrouter.ai/keys): " key || key=""; echo
  fi
  [[ -n "$key" ]] || { echo "No API key provided" >&2; exit 1; }
}

write_config() {
  mkdir -p "$CONFIG_DIR"
  if [[ -n "$url" ]]; then
    if [[ -z "$key" ]]; then read -r -s -p "Virtual key: " key || key=""; echo; fi
    [[ -n "$key" ]] || { echo "Gateway URL and key are required" >&2; exit 1; }
    [[ -n "$model" ]] || model="bharatgen-param"
    cat > "${ENV_FILE}.new" <<EOF
GOOSE_PROVIDER=litellm
GOOSE_MODEL=${model}
LITELLM_HOST=${url}
LITELLM_API_KEY=${key}
GOOSE_TELEMETRY_OFF=1
EOF
  else
    ask_key
    [[ -n "$model" ]] || model="anthropic/claude-sonnet-4"
    cat > "${ENV_FILE}.new" <<EOF
GOOSE_PROVIDER=openrouter
GOOSE_MODEL=${model}
OPENROUTER_API_KEY=${key}
GOOSE_TELEMETRY_OFF=1
EOF
  fi
  if [[ -f "$ENV_FILE" ]]; then cp "$ENV_FILE" "${ENV_FILE}.bak"; fi
  mv "${ENV_FILE}.new" "$ENV_FILE"
  chmod 600 "$ENV_FILE"
}

download_desktop() {
  echo "Downloading bg-code desktop app for ${os}..."
  asset=$(latest_release "desktop.*${os}\|${os}.*desktop" | head -1) \
    || { echo "No desktop asset published; download from https://github.com/${REPO}/releases" >&2; exit 1; }
  mkdir -p "${HOME}/Downloads"
  curl -fsSL "$asset" -o "${HOME}/Downloads/$(basename "$asset")"
  echo "Downloaded to ~/Downloads/$(basename "$asset") — run it to install"
}

if [[ "$os" == "windows" ]]; then
  echo "Installing bg-code CLI for Windows..."
  asset=$(latest_release "windows.*x86_64.*\.exe\|x86_64.*windows.*\.exe" | head -1) \
    || { echo "No Windows CLI asset published; download from https://github.com/${REPO}/releases" >&2; exit 1; }
  curl -fsSL "$asset" -o "${HOME}/.local/bin/bg-code.exe" --create-dirs
  echo "Installed to ${HOME}/.local/bin/bg-code.exe"
  write_config
  echo
  echo "Setup complete. In Git Bash run:"
  echo "  source ${ENV_FILE}"
  echo "  bg-code session"
  exit 0
fi

if [[ "${desktop:-0}" == 1 ]]; then
  download_desktop
  exit 0
fi

echo "Installing bg-code CLI..."
repo_root="$(cd "$(dirname "$0")/.." && pwd)"
if [[ ! -f "${repo_root}/crates/goose-cli/Cargo.toml" ]]; then
  if curl -fsSL "https://github.com/${REPO}/releases/download/stable/download_cli.sh" | bash; then
    repo_root=""
  else
    echo "Download failed (no published release?), building from source..."
    repo_root="$(pwd)"
  fi
fi

if [[ -n "$repo_root" ]]; then
  { [[ -f "${repo_root}/bin/activate-hermit" ]] && source "${repo_root}/bin/activate-hermit" || true; }
  command -v cargo >/dev/null || { echo "cargo not found — install rust first" >&2; exit 1; }
  (cd "$repo_root" && cargo build --release -p goose-cli --bin goose)
  bin_dir="${HOME}/.local/bin"
  mkdir -p "$bin_dir"
  mv "$repo_root/target/release/goose" "${bin_dir}/bg-code.new"
  mv "${bin_dir}/bg-code.new" "${bin_dir}/bg-code"
  echo "Installed to ${bin_dir}/bg-code"
fi

if [[ -z "$url" && -z "$key" ]]; then
  read -r -p "Configure via [1] OpenRouter key or [2] Manch gateway? [1]: " choice || true
  [[ "$choice" == "2" ]] && { read -r -p "Manch gateway URL (https://llm.<your-domain>): " url || url=""; }
fi

write_config

{ while IFS='=' read -r k v; do [[ "$k" =~ ^[A-Z_]+$ ]] && printf 'set -x %s "%s"\n' "$k" "$v"; done < "$ENV_FILE"; } > "${ENV_FILE}.fish"
chmod 600 "${ENV_FILE}.fish"

for profile in "${HOME}/.bashrc" "${HOME}/.profile" "${HOME}/.zshrc"; do
  if [[ -f "$profile" ]] && grep -qE "^(export )?(GOOSE_PROVIDER|GOOSE_MODEL|GOOSE_TELEMETRY_OFF|OPENROUTER_API_KEY|LITELLM_HOST|LITELLM_API_KEY)=" "$profile"; then
    grep -vE "^(export )?(GOOSE_PROVIDER|GOOSE_MODEL|GOOSE_TELEMETRY_OFF|OPENROUTER_API_KEY|LITELLM_HOST|LITELLM_API_KEY)=" "$profile" > "${profile}.bgclean"
    mv "${profile}.bgclean" "$profile"
    echo "Removed stale bg-code variables from ${profile}"
  fi
done

echo
echo "Setup complete. Run:"
echo "  source ${ENV_FILE}     # bash/zsh"
echo "  source ${ENV_FILE}.fish     # fish"
echo "  bg-code session"
