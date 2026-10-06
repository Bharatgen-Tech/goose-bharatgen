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
Usage: install.sh [--url <gateway-url>] [--key <virtual-key>] [--model <name>]

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
    *) echo "Unknown option: $1" >&2; usage ;;
  esac
done

os="$(uname -s | tr '[:upper:]' '[:lower:]')"
[[ "$os" == "darwin" || "$os" == "linux" ]] || { echo "Unsupported OS: $os" >&2; exit 1; }

echo "Installing bg-code CLI..."
repo_root="$(cd "$(dirname "$0")/.." && pwd)"
if [[ -f "${repo_root}/crates/goose-cli/Cargo.toml" ]]; then
  echo "Local clone found at ${repo_root}, building from source..."
  installed_from_clone=1
else
  installed_from_clone=0
  if ! curl -fsSL "https://github.com/${REPO}/releases/download/stable/download_cli.sh" | bash; then
    echo "Download failed (no published release?), building from source..."
    installed_from_clone=1
    repo_root="$(pwd)"
  fi
fi

if [[ "$installed_from_clone" == 1 ]]; then
  { [[ -f "${repo_root}/bin/activate-hermit" ]] && source "${repo_root}/bin/activate-hermit" || true; }
  command -v cargo >/dev/null || { echo "cargo not found — install rust first" >&2; exit 1; }
  (cd "$repo_root" && cargo build --release -p goose-cli --bin goose)
  bin_dir="${HOME}/.local/bin"
  mkdir -p "$bin_dir"
  mv "$repo_root/target/release/goose" "${bin_dir}/bg-code.new"
  mv "${bin_dir}/bg-code.new" "${bin_dir}/bg-code"
  echo "Installed to ${bin_dir}/bg-code"
fi

mode="openrouter"
if [[ -n "$url" ]]; then mode="gateway"; fi
mkdir -p "$CONFIG_DIR"

if [[ -z "$url" && -z "$key" ]]; then
  read -r -p "Configure via [1] OpenRouter key or [2] Manch gateway? [1]: " choice || true
  [[ "$choice" == "2" ]] && mode="gateway"
fi

if [[ "$mode" == "openrouter" ]]; then
  if [[ -z "$key" ]]; then
    read -r -s -p "OpenRouter API key (from openrouter.ai/keys): " key || true; echo
  fi
  if [[ -z "$key" ]]; then echo "No API key provided" >&2; exit 1; fi
  provider="openrouter"
  if [[ -z "$model" ]]; then
    read -r -p "Default model [anthropic/claude-sonnet-4]: " model || model=""
    model="${model:-anthropic/claude-sonnet-4}"
  fi
  cat > "${ENV_FILE}.new" <<EOF
GOOSE_PROVIDER=${provider}
GOOSE_MODEL=${model}
OPENROUTER_API_KEY=${key}
GOOSE_TELEMETRY_OFF=1
EOF
else
  if [[ -z "$url" ]]; then read -r -p "Manch gateway URL (https://llm.<your-domain>): " url || url=""; fi
  if [[ -z "$key" ]]; then read -r -s -p "Virtual key: " key || key=""; echo; fi
  if [[ -z "$url" || -z "$key" ]]; then echo "Gateway URL and key are required" >&2; exit 1; fi
  if [[ -z "$model" ]]; then model="bharatgen-param"; fi
  cat > "${ENV_FILE}.new" <<EOF
GOOSE_PROVIDER=litellm
GOOSE_MODEL=${model}
LITELLM_HOST=${url}
LITELLM_API_KEY=${key}
GOOSE_TELEMETRY_OFF=1
EOF
fi

if [[ -f "$ENV_FILE" ]]; then
  cp "$ENV_FILE" "${ENV_FILE}.bak"
  echo "Backed up existing config to ${ENV_FILE}.bak"
fi
mv "${ENV_FILE}.new" "$ENV_FILE"
chmod 600 "$ENV_FILE"

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
