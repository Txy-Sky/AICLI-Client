#!/usr/bin/env bash
# Per-item Rust acceptance tests, grouped by Cargo feature set so each set is
# compiled once. New checklist items add their filters here instead of adding
# workflow steps (workflow edits force the full A6 environment run).
#
# Usage: scripts/aicli-item-tests.sh <default|test-hooks> <evidence-dir>
set -euo pipefail

set_name=${1:?feature set: default or test-hooks}
evidence=${2:?evidence directory}
repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"
mkdir -p "$evidence"
manifest=src-tauri/Cargo.toml

# item <evidence-name> <nextest args...>: appends to <evidence-name>.txt
item() {
  local name=$1
  shift
  printf '\n==> cargo nextest run --manifest-path %s %s\n' "$manifest" "$*" | tee -a "$evidence/$name.txt"
  cargo nextest run --manifest-path "$manifest" "$@" 2>&1 | tee -a "$evidence/$name.txt"
}

case "$set_name" in
  default)
    item C1-01 aicli::lines::tests
    item C1-02 aicli::secrets::tests
    item C1-03 aicli::schema::tests
    item C1-03 database::schema::tests
    item C1-03 database::tests
    item C1-05 aicli::account::tests
    item C1-06-unit aicli::cache::tests
    item C1-07-unit aicli::catalog::tests
    item C1-08-unit aicli::profiles::tests
    item C1-09-unit aicli::tokens::tests
    item C2-01-unit aicli::runtime::tests
    item C2-02-unit aicli::alias::tests
    item C3-02-unit aicli::seed::tests
    item C3-01-unit aicli::resolver::tests
    item C3-04-unit aicli_forwarder
    item C3-05-unit aicli::errors::tests
    item C3-05-forwarding aicli_error_routing
    item C3-05-recovery aicli_error_recovery
    item C3-06-unit aicli::cooldown::tests
    item C3-06-forwarding aicli_cooldown_success
    item C3-07-forwarding aicli_timeout_
    item C3-08-unit aicli::local_auth::tests
    item C3-08-guard aicli_local_credentials
    item C3-10-unit aicli::takeover::tests
    item C3-11-unit commands::aicli::tests
    item C4-02-unit commands::aicli::tests::bind_ commands::aicli::tests::one_client_failure commands::aicli::tests::rebind_targets commands::aicli::tests::first_bind_skips commands::aicli::tests::proxy_start_failure_is_reported
    item C4-03-unit aicli::usage::tests
    item C4-04-unit aicli_route_columns_are_written provider_id_keeps_the_seed_id route_follows_the_final_provider
    item E-02-unit aicli::privacy::placeholder::tests
    item E-03-unit aicli::privacy::rules::redact::tests aicli::privacy::rules::phone aicli::privacy::rules::more aicli::privacy::rules::ids aicli::privacy::rules::known aicli::privacy::rules::rules aicli::privacy::rules::scrub
    item E-04-unit aicli::privacy::walker::tests
    ;;
  test-hooks)
    item C1-04-unit --features test-hooks aicli::newapi::tests
    item C1-06-api --features test-hooks aicli::newapi::tests
    item C3-03-routing --features test-hooks --test aicli_routing
    item C3-09-wire --features test-hooks --test aicli_routing aicli_credentials_follow_wire
    item C4-03-routing --features test-hooks --test aicli_routing bill_backfill
    ;;
  *)
    echo "unknown feature set: $set_name" >&2
    exit 2
    ;;
esac
printf 'aicli-item-tests %s: PASS\n' "$set_name"
