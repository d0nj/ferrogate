#!/usr/bin/env bash
set -euo pipefail

PORT=18080
BASE="http://127.0.0.1:${PORT}"
TMP="$(mktemp -d)"
BIN="${BIN:-target/debug/ferrogate}"
KEY="$(openssl rand -hex 32 2>/dev/null || python3 -c 'import secrets;print(secrets.token_hex(32))')"

cleanup() {
  kill "${PID:-}" 2>/dev/null || true
  sleep 0.5
  rm -rf "$TMP" 2>/dev/null || true
}
trap cleanup EXIT

if [ ! -x "$BIN" ]; then
  cargo build --locked --bin ferrogate
fi

DATABASE_URL="sqlite://${TMP}/smoke.db?mode=rwc" \
FERROGATE_ENCRYPTION_KEY="$KEY" \
HOST=127.0.0.1 PORT="$PORT" \
"$BIN" >/dev/null 2>&1 &
PID=$!

for _ in $(seq 1 50); do
  sleep 0.2
  if curl -fsS "$BASE/health" >/dev/null 2>&1; then
    break
  fi
done

fail() { echo "FAIL: $1"; exit 1; }

BODY="$(curl -fsS "$BASE/health")"
echo "$BODY" | grep -q '"status":"ok"' || fail "/health not ok: $BODY"
echo "$BODY" | grep -q '"database"' && fail "/health leaks database field"
echo "$BODY" | grep -q '"redis"' && fail "/health leaks redis field"

CODE="$(curl -s -o /dev/null -w '%{http_code}' "$BASE/metrics")"
[ "$CODE" = "401" ] || [ "$CODE" = "403" ] || fail "/metrics unauthenticated returned $CODE"

echo "PASS"
