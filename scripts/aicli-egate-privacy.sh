#!/usr/bin/env bash
# E-GATE 第 2 项：真实 Claude Code 隐私轮驱动。
#
# 形态：scripts/aicli-egate-gate（tests/aicli_egate_gate.rs，常驻测试）起
# 真实 ProxyServer（隐私开启）+ 脚本化捕获上游；本脚本用真实 `claude` CLI
# （H-02 本机）跑四轮，断言：上游只见占位符、客户端见原值、已知值记忆、
# 中断后无残留无报错、日志查不到假密钥。产出打码后的 summary.txt。
#
# 用法：bash scripts/aicli-egate-privacy.sh
set -uo pipefail

ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
STATE=$(mktemp -d "${TMPDIR:-/tmp}/aicli-egate.XXXXXX")
mkdir -p "$STATE/claude-config"
printf '{"hasCompletedOnboarding": true, "theme": "dark"}' > "$STATE/claude-config/.claude.json"
CAPTURE="$STATE/capture.jsonl"
PROXY_LOG="$STATE/proxy.log"

# 本地回环绝不走系统代理（本机默认代理 127.0.0.1:7897）
export NO_PROXY="127.0.0.1,localhost"
export no_proxy="$NO_PROXY"
unset HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy 2>/dev/null || true

errors=0
fail() { printf '[FAIL] %s\n' "$1" >> "$STATE/summary.txt"; errors=$((errors + 1)); }
pass() { printf '[ok]   %s\n' "$1" >> "$STATE/summary.txt"; }

# ---- 假密钥 / 假邮箱：运行时拼接生成，随后自检形态（无 {{ / TAG_，密钥 sk- 前缀） ----
join() { local IFS=''; printf '%s' "$*"; }
EMAIL=$(join 'lia' 'n.' 'zh' 'en' '@ga' 'te-e' 'vid' 'ence.' 'cn')
KEY=$(join 'sk-E' 'Ga' 'teK' 'ey7' 'fQ' '2mV' 'xR9' 'pL4' 'wD6' 'sTb')
KEY2=$(join 'sk-E' 'Ga' 'te2' 'nd9' 'cH8' 'jM5' 'nB3' 'kZ7' 'vQ1' 'xWd')
# 只有提示词（「数据库密码：」）才认得出的口令：裸出现时规则不遮，只能靠已知值记忆
PW=$(join 'Hn4' 'tRw' '8Zq' 'Lm2v')

for value in "$EMAIL" "$KEY" "$KEY2" "$PW"; do
  case "$value" in
    *'{{'*|*'TAG_'*) echo "self-check failed: generated value contains placeholder syntax" >&2; exit 2 ;;
  esac
done
case "$KEY $KEY2" in
  sk-*) : ;;
  *) echo "self-check failed: key shape" >&2; exit 2 ;;
esac
printf 'self-check: values generated at runtime, no {{ / TAG_ in them\n' > "$STATE/summary.txt"

# Windows（git bash）用 python 与 cygpath；Linux CI 用 python3、原路径
if python3 -c 'import sys' >/dev/null 2>&1; then PY=python3; else PY=python; fi
if command -v cygpath >/dev/null 2>&1; then STATE_W=$(cygpath -w "$STATE"); else STATE_W=$STATE; fi
info_field() { "$PY" -c 'import sys,json;print(json.load(open(sys.argv[1]))[sys.argv[2]])' "$STATE/info.json" "$1"; }

# ---- 启动门禁服务（真实代理 + 捕获上游） ----
EGATE_STATE_DIR="$STATE" \
  cargo test --manifest-path "$ROOT/src-tauri/Cargo.toml" --features test-hooks \
  --test aicli_egate_gate -- --ignored --nocapture > "$STATE/gate.log" 2>&1 &
GATE_PID=$!
cleanup() {
  touch "$STATE/stop" 2>/dev/null
  sleep 1
  kill "$GATE_PID" 2>/dev/null
}
trap cleanup EXIT

for _ in $(seq 1 240); do
  [[ -f "$STATE/info.json" ]] && break
  if ! kill -0 "$GATE_PID" 2>/dev/null; then
    echo "gate process died; see $STATE/gate.log" >&2; tail -20 "$STATE/gate.log"; exit 2
  fi
  sleep 1
done
[[ -f "$STATE/info.json" ]] || { echo "gate did not become ready in 240s" >&2; exit 2; }
PROXY_PORT=$(info_field proxy_port)
TOKEN=$(info_field token)
ALIAS=$(info_field model_alias)
PROXY="http://127.0.0.1:$PROXY_PORT"

# ---- 真实 Claude Code 公共环境 ----
run_claude() { # run_claude <output-file> <prompt...>（EGATE_FORMAT 可改输出格式）
  local out=$1; shift
  local format=${EGATE_FORMAT:-text}
  local extra=()
  [[ "$format" == stream-json ]] && extra=(--verbose)
  env \
    ANTHROPIC_BASE_URL="$PROXY" \
    ANTHROPIC_API_KEY="$TOKEN" \
    ANTHROPIC_MODEL="$ALIAS" \
    ANTHROPIC_DEFAULT_OPUS_MODEL="$ALIAS" \
    ANTHROPIC_DEFAULT_SONNET_MODEL="$ALIAS" \
    ANTHROPIC_DEFAULT_HAIKU_MODEL="$ALIAS" \
    ANTHROPIC_SMALL_FAST_MODEL="$ALIAS" \
    CLAUDE_CONFIG_DIR="$STATE/claude-config" \
    CLAUDE_DISABLE_NONESSENTIAL_TRAFFIC=1 \
    DISABLE_TELEMETRY=1 \
    DISABLE_ERROR_REPORTING=1 \
    timeout 120 claude -p "$*" --model "$ALIAS" --output-format "$format" "${extra[@]}" \
      --dangerously-skip-permissions > "$out" 2> "$out.err"
}

capture_count() { [[ -f "$CAPTURE" ]] && wc -l < "$CAPTURE" || echo 0; }

# ---- Round 0：curl 链路 sanity（非流式，验证夹具接线） ----
export STATE_W EMAIL KEY ALIAS PROXY_PORT PW
"$PY" - <<'PY0' > "$STATE/round0.status"
import json, os, urllib.request, urllib.error
state = os.environ["STATE_W"]
info = json.load(open(os.path.join(state, "info.json"), encoding="utf-8"))
body = json.dumps({"model": os.environ["ALIAS"], "max_tokens": 64, "stream": False,
    "messages": [{"role": "user", "content": [{"type": "text",
        "text": "邮箱 {} 与密钥 {}，请登记".format(os.environ["EMAIL"], os.environ["KEY"])}]}]},
    ensure_ascii=False).encode("utf-8")
req = urllib.request.Request(
    "http://127.0.0.1:{}/v1/messages".format(os.environ["PROXY_PORT"]), data=body,
    headers={"x-api-key": info["token"], "content-type": "application/json"})
try:
    r = urllib.request.urlopen(req, timeout=60)
    open(os.path.join(state, "round0.json"), "wb").write(r.read())
    print(r.status)
except urllib.error.HTTPError as e:
    open(os.path.join(state, "round0.json"), "wb").write(e.read())
    print(e.code)
PY0
if [[ "$(cat "$STATE/round0.status")" == "200" ]]; then
  grep -q "$EMAIL" "$STATE/round0.json" && ! grep -Eq '\{\{[A-Z][A-Z0-9_]{1,23}_[a-z0-9]{8}\}\}' "$STATE/round0.json" \
    && pass "round0 curl: 200, restored original, no placeholder leaked" \
    || fail "round0 curl: response not restored: $(head -c 300 "$STATE/round0.json")"
else
  fail "round0 curl: status $(cat "$STATE/round0.status")"
fi
BASE_COUNT=$(capture_count)
if (( BASE_COUNT > 0 )) && ! grep -q "$EMAIL" "$CAPTURE" && ! grep -q "$KEY" "$CAPTURE"; then
  pass "round0 upstream capture: masked (no raw values)"
else
  fail "round0 upstream capture contains raw values or empty"
fi
R0_PLACEHOLDER=$(grep -o '{{[A-Z_]*_[a-z0-9]*}}' "$CAPTURE" | head -1)

# ---- 对照：口令第一次裸出现，规则单独不遮（证明 Round 2 靠的是已知值记忆） ----
"$PY" - <<'PYC' > "$STATE/control.status"
import json, os, urllib.request
state = os.environ["STATE_W"]
info = json.load(open(os.path.join(state, "info.json"), encoding="utf-8"))
body = json.dumps({"model": os.environ["ALIAS"], "max_tokens": 64, "stream": False,
    "messages": [{"role": "user", "content": "把连接参数 {} 填进配置".format(os.environ["PW"])}]},
    ensure_ascii=False).encode("utf-8")
req = urllib.request.Request(
    "http://127.0.0.1:{}/v1/messages".format(os.environ["PROXY_PORT"]), data=body,
    headers={"x-api-key": info["token"], "content-type": "application/json"})
print(urllib.request.urlopen(req, timeout=60).status)
PYC
CONTROL_END=$(capture_count)
if [[ "$(cat "$STATE/control.status")" == "200" ]] \
  && sed -n "$((BASE_COUNT + 1)),${CONTROL_END}p" "$CAPTURE" | grep -q "$PW"; then
  pass "control: bare password not masked by rules on first sight"
else
  fail "control: bare password was masked (or request failed) before it was ever cued"
fi
BASE_COUNT=$CONTROL_END

# ---- Round 1：真实 Claude Code，工具调用 + thinking 多轮 ----
EGATE_FORMAT=stream-json run_claude "$STATE/round1.out" \
  "EGATE-TOOL 请登记：我的邮箱是 $EMAIL，API 密钥是 $KEY，数据库密码：$PW"
R1_START=$BASE_COUNT
if grep -q "$EMAIL" "$STATE/round1.out" && grep -q "$KEY" "$STATE/round1.out"; then
  pass "round1 claude: saw original values (response restore)"
else
  fail "round1 claude: originals missing: $(head -c 300 "$STATE/round1.out")"
fi
if ! grep -Eq '\{\{[A-Z][A-Z0-9_]{1,23}_[a-z0-9]{8}\}\}' "$STATE/round1.out" && ! grep -q 'TAG_' "$STATE/round1.out"; then
  pass "round1 claude: no placeholder leaked to client"
else
  fail "round1 claude: placeholder leaked: $(head -c 300 "$STATE/round1.out")"
fi
# 客户端侧（stream-json）：thinking、工具参数、工具执行结果都是原值
export KEY EMAIL
"$PY" - "$STATE/round1.out" > "$STATE/round1.client" <<'PYJ'
import json, os, re, sys
key, email = os.environ["KEY"], os.environ["EMAIL"]
ph = re.compile(r"\{\{[A-Z][A-Z0-9_]{0,23}_[a-z0-9]{8}\}\}")
thinking, tool_input, tool_result = [], [], []
for line in open(sys.argv[1], encoding="utf-8"):
    try:
        event = json.loads(line)
    except ValueError:
        continue
    message = event.get("message") or {}
    for block in message.get("content") or []:
        if not isinstance(block, dict):
            continue
        kind = block.get("type")
        if kind == "thinking":
            thinking.append(block.get("thinking", ""))
        elif kind == "tool_use":
            tool_input.append(json.dumps(block.get("input"), ensure_ascii=False))
        elif kind == "tool_result":
            content = block.get("content")
            tool_result.append(content if isinstance(content, str) else json.dumps(content, ensure_ascii=False))
def check(name, texts, value):
    joined = "\n".join(texts)
    ok = bool(texts) and value in joined and not ph.search(joined)
    print(("ok " if ok else "bad ") + name)
check("thinking", thinking, email)
check("tool_input", tool_input, key)
check("tool_result", tool_result, "egate-tool-ok " + key)
PYJ
for part in thinking tool_input tool_result; do
  if grep -qx "ok $part" "$STATE/round1.client"; then
    pass "round1 client: $part carries the original value, no placeholder"
  else
    fail "round1 client: $part not restored ($(grep " $part" "$STATE/round1.client"))"
  fi
done
R1_END=$(capture_count)
sed -n "$((R1_START + 1)),${R1_END}p" "$CAPTURE" > "$STATE/round1.captures"
if grep -q "$EMAIL" "$STATE/round1.captures" || grep -q "$KEY" "$STATE/round1.captures" \
  || grep -q "$PW" "$STATE/round1.captures"; then
  fail "round1 upstream capture: raw values leaked"
else
  pass "round1 upstream capture: all masked"
fi
# 第二跳：工具执行结果回到上游时，密钥又被遮回占位符
if grep -qF 'egate-tool-ok {{API_KEY_' "$STATE/round1.captures"; then
  pass "round1 upstream: tool_result re-masked on the way back"
else
  fail "round1 upstream: no masked tool_result in second hop"
fi


# ---- Round 2：口令不带任何提示词裸出现 → 只能靠已知值记忆遮 ----
# （对照轮已证明规则单独不遮这个形态；邮箱/sk- 密钥规则本身就认得，不能用来证明记忆）
run_claude "$STATE/round2.out" "把连接参数 $PW 填进配置"
R2_START=$R1_END
R2_END=$(capture_count)
sed -n "$((R2_START + 1)),${R2_END}p" "$CAPTURE" > "$STATE/round2.captures"
if grep -q "$PW" "$STATE/round2.captures"; then
  fail "round2 upstream capture: bare password leaked (known-value memory broken)"
else
  pass "round2 upstream capture: bare password masked (known-value memory)"
fi
# 第 2 轮消息里只有这一个敏感值：出现的占位符都必须是第 1 轮凭提示词遮出的那个
R2_PHS=$(grep -oE '\{\{[A-Z][A-Z0-9_]{0,23}_[a-z0-9]{8}\}\}' "$STATE/round2.captures" | sort -u)
r2_ok=1
[[ -n "$R2_PHS" ]] || r2_ok=0
for ph in $R2_PHS; do grep -qF "$ph" "$STATE/round1.captures" || r2_ok=0; done
if (( r2_ok )); then
  pass "round2 placeholder identical to round1 ($(echo $R2_PHS))"
else
  fail "round2 placeholders not from round1: ${R2_PHS:-none}"
fi

# ---- Round 3：流式中途中断（杀掉真实客户端 ≈ Esc 中断） ----
env \
  ANTHROPIC_BASE_URL="$PROXY" ANTHROPIC_API_KEY="$TOKEN" \
  ANTHROPIC_MODEL="$ALIAS" ANTHROPIC_DEFAULT_OPUS_MODEL="$ALIAS" \
  ANTHROPIC_DEFAULT_SONNET_MODEL="$ALIAS" ANTHROPIC_DEFAULT_HAIKU_MODEL="$ALIAS" \
  ANTHROPIC_SMALL_FAST_MODEL="$ALIAS" \
  CLAUDE_CONFIG_DIR="$STATE/claude-config" \
  CLAUDE_DISABLE_NONESSENTIAL_TRAFFIC=1 DISABLE_TELEMETRY=1 \
  claude -p "EGATE-INTERRUPT 慢慢回复，不要急。顺便记下备用密钥 $KEY2" \
    --model "$ALIAS" --output-format text --dangerously-skip-permissions \
  > "$STATE/round3.out" 2> "$STATE/round3.err" &
CLAUDE_PID=$!
sleep 5
kill "$CLAUDE_PID" 2>/dev/null
sleep 2
R3_START=$R2_END
R3_END=$(capture_count)
sed -n "$((R3_START + 1)),${R3_END}p" "$CAPTURE" > "$STATE/round3.captures"
if grep -q "$KEY2" "$STATE/round3.captures"; then
  fail "round3 upstream capture: raw key leaked"
else
  pass "round3 upstream capture: masked despite interruption"
fi
if grep -qiE '\bERROR\b' "$PROXY_LOG"; then
  fail "round3 proxy log has ERROR lines: $(grep -i -m3 '\bERROR\b' "$PROXY_LOG" | head -3)"
else
  pass "round3 proxy log: no ERROR after interrupted stream"
fi

# ---- Round 4：中断后正常消息 → 无占位符残留 ----
run_claude "$STATE/round4.out" "报一下 $KEY2 的登记结果"
R4_START=$R3_END
R4_END=$(capture_count)
sed -n "$((R4_START + 1)),${R4_END}p" "$CAPTURE" > "$STATE/round4.captures"
if grep -q "$KEY2" "$STATE/round4.out"; then
  pass "round4 claude: saw original value (restore intact after interruption)"
else
  fail "round4 claude: original missing: $(head -c 300 "$STATE/round4.out")"
fi
if ! grep -Eq '\{\{[A-Z][A-Z0-9_]{1,23}_[a-z0-9]{8}\}\}' "$STATE/round4.out" && ! grep -q 'TAG_' "$STATE/round4.out"; then
  pass "round4 claude: no placeholder residue"
else
  fail "round4 claude: placeholder residue: $(head -c 300 "$STATE/round4.out")"
fi
if grep -q "$KEY2" "$STATE/round4.captures"; then
  fail "round4 upstream capture: raw key leaked"
else
  pass "round4 upstream capture: masked"
fi

# ---- 全局断言：mock/日志查不到任何植入假密钥 ----
# 对照轮的口令是有意未遮的，只查对照轮之后的记录
if grep -q "$EMAIL" "$CAPTURE" || grep -qE "$KEY|$KEY2" "$CAPTURE" \
  || tail -n +$((R1_START + 1)) "$CAPTURE" | grep -q "$PW"; then
  fail "GLOBAL: raw values found in capture.jsonl (mock account log)"
else
  pass "GLOBAL: capture.jsonl has no planted secrets"
fi
if grep -qE "$EMAIL|$KEY|$KEY2|$PW" "$PROXY_LOG"; then
  fail "GLOBAL: raw values found in proxy.log"
else
  pass "GLOBAL: proxy.log has no planted secrets"
fi
TOTAL=$(capture_count)
printf 'capture entries: %s; claude version: %s\n' "$TOTAL" "$(claude --version 2>/dev/null | head -1)" >> "$STATE/summary.txt"

# ---- 收尾：停门禁，输出打码摘要 ----
touch "$STATE/stop"
for _ in $(seq 1 30); do kill -0 "$GATE_PID" 2>/dev/null || break; sleep 1; done

for f in round1.out round2.out round4.out; do
  sed -e "s|$EMAIL|***EMAIL***|g" -e "s|$KEY2|***KEY2***|g" -e "s|$KEY|***KEY***|g" \
    -e "s|$PW|***PW***|g" "$STATE/$f" > "$STATE/$f.masked" 2>/dev/null || true
done
printf '\nstate dir: %s\n' "$STATE"
cat "$STATE/summary.txt"
# CI：打码后的摘要与客户端输出存进证据目录（原始 capture/proxy.log 含占位符原文映射，不存）
if [[ -n "${EGATE_EVIDENCE_DIR:-}" ]]; then
  mkdir -p "$EGATE_EVIDENCE_DIR"
  cp "$STATE/summary.txt" "$EGATE_EVIDENCE_DIR/E-GATE-privacy-summary.txt"
  for f in round1.out round2.out round4.out; do
    [[ -f "$STATE/$f.masked" ]] && cp "$STATE/$f.masked" "$EGATE_EVIDENCE_DIR/E-GATE-$f.masked.txt"
  done
fi
if (( errors > 0 )); then
  printf '\n%d assertion(s) failed\n' "$errors"
  exit 1
fi
printf '\nall assertions passed\n'
