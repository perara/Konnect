#!/usr/bin/env bash
# End-to-end check that the Docker image actually serves MCP over both
# transports. Requires Docker; run from the repo root:  docker/smoke-test.sh
#
# Not part of `cargo test` (CI may not have Docker). The transport logic itself
# is covered by crates/konnect/tests/protocol_{http,stdio}.rs; this proves the
# packaged binary starts and answers inside the container.
set -euo pipefail

# Stop git-bash (MSYS) from rewriting a leading-slash arg like the container
# path /etc/konnect/... into a Windows path. No-op on Linux/macOS.
export MSYS_NO_PATHCONV=1

IMAGE="${IMAGE:-konnect:smoke}"
PORT="${PORT:-3999}"
NAME="konnect-smoke-$$"

cleanup() { docker rm -f "$NAME" >/dev/null 2>&1 || true; }
trap cleanup EXIT

echo "== building image =="
docker build -t "$IMAGE" .

echo "== stdio transport =="
# Pipe one initialize request in; expect a JSON-RPC response naming the server.
INIT='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}'
OUT=$(printf '%s\n' "$INIT" | docker run --rm -i "$IMAGE")
echo "$OUT" | grep -q '"name":"konnect"' \
    && echo "  ok: stdio handshake returned serverInfo.name=konnect" \
    || { echo "  FAIL: unexpected stdio response: $OUT"; exit 1; }

echo "== http transport =="
docker run -d --rm --name "$NAME" -p "${PORT}:3000" \
    "$IMAGE" --config /etc/konnect/konnect.toml >/dev/null

# Wait for /health.
for _ in $(seq 1 30); do
    if curl -fsS "http://localhost:${PORT}/health" >/dev/null 2>&1; then break; fi
    sleep 1
done
curl -fsS "http://localhost:${PORT}/health" | grep -q ok \
    && echo "  ok: /health responded" \
    || { echo "  FAIL: /health never came up"; exit 1; }

RESP=$(curl -fsS -H 'Content-Type: application/json' \
    -d "$INIT" "http://localhost:${PORT}/mcp")
echo "$RESP" | grep -q '"name":"konnect"' \
    && echo "  ok: http handshake returned serverInfo.name=konnect" \
    || { echo "  FAIL: unexpected http response: $RESP"; exit 1; }

echo "== all smoke checks passed =="
