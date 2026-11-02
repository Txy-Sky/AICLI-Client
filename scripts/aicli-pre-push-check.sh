#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

# Formatting only: no local compilation, unit tests or client configuration writes.
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
check_home=$(mktemp -d "${TMPDIR:-/tmp}/aicli-format-home.XXXXXX")
trap 'rm -rf -- "$check_home"' EXIT
export HOME="$check_home"
export CC_SWITCH_TEST_HOME="$check_home"
export XDG_CONFIG_HOME="$check_home/.config"
export XDG_DATA_HOME="$check_home/.local/share"
export XDG_CACHE_HOME="$check_home/.cache"

prettier_bin="${AICLI_PRETTIER_BIN:-$repo_root/node_modules/.bin/prettier}"
if [ ! -x "$prettier_bin" ]; then
  prettier_bin=$(command -v prettier) || {
    echo 'Install the lockfile version of Prettier outside the repo and set AICLI_PRETTIER_BIN.' >&2
    exit 1
  }
fi

echo '==> cargo fmt --check --manifest-path src-tauri/Cargo.toml'
cargo fmt --check --manifest-path src-tauri/Cargo.toml
echo '==> prettier --check (source and acceptance YAML)'
"$prettier_bin" --check 'src/**/*.{js,jsx,ts,tsx,css,json}' \
  .github/workflows/aicli-acceptance.yml .github/actions/aicli-setup/action.yml
echo '==> workflow YAML syntax'
python3 scripts/aicli-check-workflow-yaml.py
echo 'Pre-push formatting and YAML checks: PASS'
