#!/usr/bin/env bash
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
runtime="$repo/tmp/lan-client"
device_host=${REMARKABLE_HOST:-rmpp-wifi}
remote=/home/root/smart-remarkable
port=8765
response_mode=${REMARKABLE_RESPONSE_MODE:-ink}
socket="$runtime/ssh.sock"
mkdir -p "$runtime"
chmod 700 "$runtime"
umask 077

case "${1:-status}" in
  deploy)
    test -x "$repo/target/aarch64-unknown-linux-gnu/release/smart_remarkable"
    # Stage before replacing executables so deploy cannot truncate a running binary.
    tar -czf - -C "$repo/target/aarch64-unknown-linux-gnu/release" smart_remarkable capture \
      -C "$repo/prompts" selection_concise.json selection_concise_ink.json | \
      ssh -o BatchMode=yes "$device_host" \
        "mkdir -p $remote/incoming && tar -xzf - -C $remote/incoming && chmod 700 $remote/incoming/smart_remarkable $remote/incoming/capture && mv $remote/incoming/smart_remarkable $remote/smart_remarkable && mv $remote/incoming/capture $remote/capture && mv $remote/incoming/selection_concise.json $remote/selection_concise.json && mv $remote/incoming/selection_concise_ink.json $remote/selection_concise_ink.json"
    ;;
  start)
    case "$response_mode" in
      ink) device_output_args='--prompt selection_concise_ink.json --no-keyboard' ;;
      text) device_output_args='--prompt selection_concise.json' ;;
      *) echo 'REMARKABLE_RESPONSE_MODE must be ink or text' >&2; exit 2 ;;
    esac
    if [[ -f "$runtime/bridge.pid" ]] && kill -0 "$(cat "$runtime/bridge.pid")" 2>/dev/null; then
      echo 'Bridge is already running. Use status or stop first.' >&2
      exit 1
    fi
    python3 - "$runtime" <<'PY'
import os, pathlib, secrets, socket, sys
root = pathlib.Path(sys.argv[1])
with socket.socket() as probe:
    probe.bind(('127.0.0.1', 8765))
token = root / 'bridge.token'
if not token.exists():
    token.write_text(secrets.token_urlsafe(36) + '\n')
    token.chmod(0o600)
env = root / 'device.env'
env.write_text('OPENAI_API_KEY=' + token.read_text().strip() + '\n')
env.chmod(0o600)
PY
    scp -O "$runtime/device.env" "$device_host:$remote/device.env"
    ssh -o BatchMode=yes "$device_host" "chmod 600 $remote/device.env"
    bridge_args=(--token-file "$runtime/bridge.token" --port "$port" --mode "$response_mode")
    if [[ -n "${REMARKABLE_CODEX_MODEL:-}" ]]; then
      bridge_args+=(--model "$REMARKABLE_CODEX_MODEL")
    fi
    python3 - "$repo" "$runtime" "${bridge_args[@]}" <<'PY'
import pathlib, subprocess, sys
repo, runtime = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
with (runtime / 'bridge.log').open('a') as log:
    process = subprocess.Popen([sys.executable, str(repo / 'bridge/codex_bridge.py'), *sys.argv[3:]],
                               cwd=repo, stdin=subprocess.DEVNULL, stdout=log,
                               stderr=subprocess.STDOUT, start_new_session=True)
(runtime / 'bridge.pid').write_text(str(process.pid) + '\n')
PY
    rollback() {
      kill "$(cat "$runtime/bridge.pid")" 2>/dev/null || true
      ssh -S "$socket" -O exit "$device_host" 2>/dev/null || true
      ssh -o BatchMode=yes -o ConnectTimeout=5 "$device_host" \
        'systemctl stop smart-remarkable-lan.service' 2>/dev/null || true
    }
    trap rollback ERR
    python3 - <<'PY'
import time, urllib.request
for i in range(30):
    try:
        with urllib.request.urlopen('http://127.0.0.1:8765/health', timeout=1) as r:
            assert r.status == 200
        break
    except Exception:
        if i == 29:
            raise
        time.sleep(0.2)
PY
    if ssh -S "$socket" -O check "$device_host" 2>/dev/null; then
      ssh -S "$socket" -O exit "$device_host"
    fi
    ssh -M -S "$socket" -fNT -o BatchMode=yes -o ExitOnForwardFailure=yes \
      -R "127.0.0.1:$port:127.0.0.1:$port" "$device_host"
    ssh -o BatchMode=yes "$device_host" \
      "systemd-run --unit=smart-remarkable-lan --uid=root --collect --property=WorkingDirectory=$remote --property=EnvironmentFile=$remote/device.env $remote/smart_remarkable --engine openai --engine-base-url http://127.0.0.1:$port --model codex --select-mode --trigger-corner four-finger $device_output_args --no-draw-progress"
    sleep 1
    ssh -o BatchMode=yes "$device_host" 'systemctl is-active --quiet smart-remarkable-lan.service'
    trap - ERR
    echo 'Ready: lasso a question, then tap with four fingers. Use a disposable notebook for initial tests.'
    ;;
  stop)
    ssh -o BatchMode=yes -o ConnectTimeout=5 "$device_host" \
      'systemctl stop smart-remarkable-lan.service' || echo 'Could not stop tablet service; check device connectivity.' >&2
    ssh -S "$socket" -O exit "$device_host" 2>/dev/null || true
    if [[ -f "$runtime/bridge.pid" ]]; then
      bridge_pid=$(cat "$runtime/bridge.pid")
      if ps -p "$bridge_pid" -o command= | grep -Fq "$repo/bridge/codex_bridge.py"; then
        kill "$bridge_pid"
      fi
      rm -f "$runtime/bridge.pid"
    fi
    ;;
  capture)
    ssh -o BatchMode=yes "$device_host" "$remote/capture /tmp/smart-remarkable-capture.png"
    scp -O "$device_host:/tmp/smart-remarkable-capture.png" "$runtime/capture.png"
    echo "$runtime/capture.png"
    ;;
  status)
    curl --fail --silent --show-error "http://127.0.0.1:$port/health" || true
    ssh -S "$socket" -O check "$device_host" 2>/dev/null || true
    ssh -o BatchMode=yes -o ConnectTimeout=5 "$device_host" \
      'systemctl is-active xochitl; systemctl is-active smart-remarkable-lan.service' || true
    ;;
  *)
    echo 'Usage: scripts/lan-client.sh {deploy|start|stop|capture|status}' >&2
    exit 2
    ;;
esac
