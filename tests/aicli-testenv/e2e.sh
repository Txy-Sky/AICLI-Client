#!/usr/bin/env bash
# K-01: the whole client flow against the running, seeded rc.34 compose
# environment: bind -> catalog -> route profile -> token -> request -> bill ->
# unbind (tests/test_e2e.py, which also runs the K-02 secret scan and the
# K-03 ~/.cc-switch check), with MySQL's general query log on to show that
# only new-api ever connected to the database.
#
#   AICLI_COMPOSE_PROJECT=<project> tests/aicli-testenv/e2e.sh <evidence-dir>
#
# Needs the test-hooks probe built (cargo build --features test-hooks
# --example aicli-e2e-probe). Never points at a public origin.
set -euo pipefail

evidence=${1:?evidence directory}
project=${AICLI_COMPOSE_PROJECT:?compose project name}
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
compose=(docker compose -p "$project" -f "$root/tests/aicli-testenv/docker-compose.yml")
mkdir -p "$evidence"
log_file=/var/lib/mysql/aicli-k01-general.log

# Turn the general log on. This admin session connects before logging starts,
# so the log holds only connections made afterwards.
"${compose[@]}" exec -T mysql mysql -uroot -paicli_mysql --silent -e \
  "SET GLOBAL general_log_file='$log_file'; SET GLOBAL general_log=ON;" 2>/dev/null
# new-api pools its connections; restart it so it reconnects under the log.
"${compose[@]}" restart new-api >/dev/null
for _ in $(seq 1 60); do
  if curl -fsS http://127.0.0.1:13000/api/status | grep -q '"success":true'; then break; fi
  sleep 2
done

set +e
PYTHONPATH="$root/tests/aicli-testenv" python3 -m pytest -q -s --tb=short \
  "$root/tests/aicli-testenv/tests/test_e2e.py" 2>&1 | tee "$evidence/K-01.txt"
status=${PIPESTATUS[0]}
set -e

# Read the log by copying the file out: no new database session.
mysql_id=$("${compose[@]}" ps -q mysql)
newapi_id=$("${compose[@]}" ps -q new-api)
newapi_ip=$(docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}} {{end}}' "$newapi_id")
docker cp "$mysql_id:$log_file" "$evidence/.general.log" >/dev/null
set +e
python3 - "$evidence/.general.log" "$newapi_ip" <<'PY' | tee -a "$evidence/K-01.txt"
import re
import sys

path, newapi_ips = sys.argv[1], set(sys.argv[2].split())
hosts = {}
with open(path, encoding="utf-8", errors="replace") as handle:
    for line in handle:
        match = re.search(r"\bConnect\s+\S+@(\S+)\s+on\b", line)
        if match:
            hosts[match.group(1)] = hosts.get(match.group(1), 0) + 1
# The mysql container's own healthcheck (mysqladmin ping -h 127.0.0.1) connects
# over its loopback. The client runs on the host and the MySQL port is not
# published, so any connection of the client's would show the network's gateway.
loopback = {"127.0.0.1", "localhost", "::1"}
others = {h: n for h, n in hosts.items() if h not in newapi_ips | loopback}
print(f"K-01 MySQL general log: connections by host {hosts}; new-api is {sorted(newapi_ips)}; loopback is the mysql container's healthcheck")
if not any(h in newapi_ips for h in hosts):
    print("K-01 MySQL general log: FAIL no connection logged (log not active?)")
    sys.exit(1)
if others:
    print(f"K-01 MySQL general log: FAIL connections not from new-api: {others}")
    sys.exit(1)
print("K-01 MySQL general log: PASS only new-api connected to the database")
PY
log_status=${PIPESTATUS[0]}
set -e
rm -f "$evidence/.general.log"
if [ "$status" -ne 0 ] || [ "$log_status" -ne 0 ]; then
  echo "K-01: FAIL (flow status $status, database log status $log_status)" | tee -a "$evidence/K-01.txt"
  exit 1
fi
echo "K-01: PASS" | tee -a "$evidence/K-01.txt"
