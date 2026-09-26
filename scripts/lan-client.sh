#!/usr/bin/env bash
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
runtime="$repo/tmp/lan-client"
device_host=${REMARKABLE_HOST:-rmpp-wifi}
remote=/home/root/smart-remarkable
mkdir -p "$runtime"
chmod 700 "$runtime"
umask 077

action=${1:-status}
case "$action" in
  deploy)
    test -x "$repo/target/aarch64-unknown-linux-gnu/release/smart_remarkable"
    # Stage before replacing executables so deploy cannot truncate a running binary.
    tar -czf - -C "$repo/target/aarch64-unknown-linux-gnu/release" smart_remarkable capture \
      -C "$repo/prompts" selection_concise.json selection_concise_ink.json \
      -C "$repo/device/settings" Settings.qml AssistantButton.qml settings.qmd activate.sh | \
      ssh -o BatchMode=yes "$device_host" \
        "mkdir -p $remote/incoming && tar -xzf - -C $remote/incoming && chmod 700 $remote/incoming/smart_remarkable $remote/incoming/capture && mv $remote/incoming/smart_remarkable $remote/smart_remarkable && mv $remote/incoming/capture $remote/capture && mv $remote/incoming/selection_concise.json $remote/selection_concise.json && mv $remote/incoming/selection_concise_ink.json $remote/selection_concise_ink.json && mv $remote/incoming/Settings.qml $remote/Settings.qml && mv $remote/incoming/AssistantButton.qml $remote/AssistantButton.qml && mv $remote/incoming/settings.qmd $remote/settings.qmd && mv $remote/incoming/activate.sh $remote/settings-ui.sh && chmod 700 $remote/settings-ui.sh"
    ;;
  start|stop|status|install-autostart|uninstall-autostart)
    exec python3 "$repo/bridge/lan_service.py" "$action"
    ;;
  capture)
    ssh -o BatchMode=yes "$device_host" "$remote/capture /tmp/smart-remarkable-capture.png"
    scp -O "$device_host:/tmp/smart-remarkable-capture.png" "$runtime/capture.png"
    echo "$runtime/capture.png"
    ;;
  *)
    echo 'Usage: scripts/lan-client.sh {deploy|start|stop|capture|status|install-autostart|uninstall-autostart}' >&2
    exit 2
    ;;
esac
