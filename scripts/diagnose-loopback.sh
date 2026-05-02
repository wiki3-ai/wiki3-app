#!/usr/bin/env bash
# diagnose-loopback.sh — gather evidence to decide whether
# Apple Container's publish-proxy is broken on this machine
# or something host-specific is filtering loopback.
#
# Usage:
#   ./diagnose-loopback.sh [HOST_PORT]            # auto-detects HOST_PORT from `container ls`
#   ./diagnose-loopback.sh 8000
#
# Run it on the affected machine while a wiki3 container is up
# and serving (i.e. the dashboard link is grey but the wiki is
# actually running). Output is plain text on stdout — pipe to a
# file or paste back into the chat.
#
# Requires: container, curl, nc, python3, sudo (for tcpdump/pfctl).
# tcpdump and pfctl steps are skipped if sudo is unavailable.

set -u

PORT="${1:-}"
ALT_PORT="${ALT_PORT:-18901}"

heading() {
    printf '\n========== %s ==========\n' "$*"
}

run() {
    printf '\n$ %s\n' "$*"
    eval "$@" 2>&1
    printf '(exit=%s)\n' "$?"
}

heading "system + tools"
run "sw_vers"
run "uname -a"
run "container --version"
run "which -a container"
run "container system status"

heading "running containers"
run "container ls -a"

if [[ -z "$PORT" ]]; then
    PORT="$(container ls --format json 2>/dev/null \
        | python3 -c '
import json, sys
try:
    arr = json.load(sys.stdin)
except Exception:
    sys.exit(0)
for obj in arr:
    cfg = obj.get("configuration", obj)
    for p in cfg.get("publishedPorts", []) or []:
        hp = p.get("hostPort")
        if hp:
            print(hp)
            sys.exit(0)
' || true)"
fi
if [[ -z "$PORT" ]]; then
    PORT=8000
    echo "No host port detected; defaulting to $PORT" >&2
fi
echo "Using HOST_PORT=$PORT, ALT_PORT=$ALT_PORT"

CONTAINER_NAME="$(container ls --format json 2>/dev/null \
    | python3 -c "
import json, sys
try:
    arr = json.load(sys.stdin)
except Exception:
    sys.exit(0)
for obj in arr:
    n = obj.get('name') or obj.get('configuration',{}).get('id')
    if n:
        print(n); sys.exit(0)
" || true)"
echo "Detected container name: ${CONTAINER_NAME:-<none>}"

heading "container inspect (network section)"
if [[ -n "${CONTAINER_NAME:-}" ]]; then
    run "container inspect '$CONTAINER_NAME' | python3 -m json.tool | grep -E 'ipv4Address|publishedPorts|hostAddress|hostPort|containerPort' -A0 -B0"
fi

heading "host listening sockets on \$PORT"
run "sudo lsof -nP -iTCP:$PORT -sTCP:LISTEN || true"
run "netstat -an -p tcp | grep -E '\\.$PORT|\\.$ALT_PORT' || true"

heading "loopback probes (the failing path)"
run "curl -v --max-time 3 http://127.0.0.1:$PORT/"
run "curl -v --max-time 3 http://localhost:$PORT/"
run "printf 'GET / HTTP/1.0\\r\\n\\r\\n' | nc -v -w 3 127.0.0.1 $PORT"

if [[ -n "${CONTAINER_NAME:-}" ]]; then
    IPV4="$(container inspect "$CONTAINER_NAME" \
        | python3 -c "
import json, sys
arr = json.load(sys.stdin)
for obj in arr:
    for n in obj.get('networks', []) or []:
        a = n.get('ipv4Address')
        if a:
            print(a.split('/')[0]); sys.exit(0)
" 2>/dev/null || true)"
    if [[ -n "$IPV4" ]]; then
        heading "direct vmnet probe (the working path)"
        run "curl -v --max-time 3 http://$IPV4:$PORT/"
    fi
fi

heading "control: our own loopback listener (no Apple Container)"
# Spin up a python http.server on a different port, hit it, kill it.
( python3 -m http.server --bind 127.0.0.1 "$ALT_PORT" >/dev/null 2>&1 ) &
PY_PID=$!
sleep 1
run "curl -v --max-time 3 http://127.0.0.1:$ALT_PORT/"
run "printf 'GET / HTTP/1.0\\r\\n\\r\\n' | nc -v -w 3 127.0.0.1 $ALT_PORT"
kill "$PY_PID" 2>/dev/null || true
wait "$PY_PID" 2>/dev/null || true

heading "packet capture: who is sending the RST?"
if command -v tcpdump >/dev/null 2>&1 && sudo -n true 2>/dev/null; then
    # Capture in the background, hit the port once, stop.
    sudo tcpdump -i lo0 -n -c 30 -tttt "tcp port $PORT" >/tmp/diagnose-tcpdump.txt 2>&1 &
    TCP_PID=$!
    sleep 1
    curl --max-time 3 -s -o /dev/null "http://127.0.0.1:$PORT/" || true
    sleep 1
    sudo kill "$TCP_PID" 2>/dev/null || true
    wait "$TCP_PID" 2>/dev/null || true
    echo
    echo "----- tcpdump on lo0 (port $PORT) -----"
    cat /tmp/diagnose-tcpdump.txt
    rm -f /tmp/diagnose-tcpdump.txt
else
    echo "(skipping tcpdump — needs sudo without prompt; rerun with: sudo $0 $PORT)"
fi

heading "packet filter rules"
if sudo -n true 2>/dev/null; then
    run "sudo pfctl -si"
    run "sudo pfctl -sr 2>&1 | head -40"
else
    echo "(skipping pfctl — needs sudo)"
fi

heading "network filter / VPN / EDR processes"
run "ps -axo pid,comm | grep -Ei 'forti|zscaler|netskope|crowdstrike|cisco|anyconnect|globalprotect|umbrella|cloudflare-warp|tailscale|cylance|sentinelone|jamf|carbonblack|splashtop' | grep -v grep || true"

heading "launchd entries mentioning container"
run "launchctl list 2>&1 | grep -i container || true"
run "ls -la /Library/LaunchDaemons /Library/LaunchAgents ~/Library/LaunchAgents 2>/dev/null | grep -i container || true"

heading "interfaces + routes"
run "ifconfig | grep -E '^[a-z0-9]+:|inet ' | grep -v inet6"
run "netstat -rn -f inet | head -30"

heading "done"
echo "Paste this entire output back into the chat."
