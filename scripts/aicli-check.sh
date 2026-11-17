#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

# Keep the installed toolchain accessible while isolating all application data.
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
test_home=$(mktemp -d "${TMPDIR:-/tmp}/aicli-check-home.XXXXXX")
trap 'rm -rf -- "$test_home"' EXIT
export HOME="$test_home"
export CC_SWITCH_TEST_HOME="$test_home"
export XDG_CONFIG_HOME="$test_home/.config"
export XDG_DATA_HOME="$test_home/.local/share"
export XDG_CACHE_HOME="$test_home/.cache"

# 本机 Windows 未配页面文件：24 核默认并行 rustc 会耗尽提交内存
# （mmap rlib 报 os error 1455）。限制构建并行度，Linux CI 不受影响。
if [[ "$(uname -s)" == *_NT-* ]]; then
  export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}"
fi

run() {
  printf '\n==>'
  printf ' %q' "$@"
  printf '\n'
  "$@"
}

run pnpm install --frozen-lockfile
run pnpm typecheck
run pnpm format:check
run pnpm test:unit
run pnpm build:renderer
run cargo fmt --check --manifest-path src-tauri/Cargo.toml
run cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
run cargo nextest run --manifest-path src-tauri/Cargo.toml
