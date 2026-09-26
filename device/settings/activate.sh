#!/bin/sh
# One-time UI activation, after the notebook has saved and the listener is idle.
# Requires the official aarch64 XOVI + qt-resource-rebuilder distribution.
set -eu
root=/home/root/smart-remarkable
xovi=/home/root/xovi
conf=/run/systemd/system/xochitl.service.d/91-smart-remarkable-settings.conf
hashtab=$xovi/exthome/qt-resource-rebuilder/hashtab
stamp=$root/settings-firmware.sha256
current=$(sha256sum /usr/bin/xochitl | cut -d ' ' -f 1)
if [ "${1:-}" = restore ]; then
    [ -f "$root/settings-enabled" ] || exit 0
    [ -f "$conf" ] && exit 0
    [ -f "$stamp" ] && [ "$(cat "$stamp")" = "$current" ] || exit 0
fi
if [ "${1:-}" = disable ]; then
    rm -f "$conf" "$root/settings-enabled"
    systemctl daemon-reload
    systemctl restart xochitl
    exit 0
fi
test -f "$xovi/xovi.so"
test -f "$xovi/extensions.d/qt-resource-rebuilder.so"
test -f "$root/Settings.qml"
buildpid=
rollback() {
    [ -z "$buildpid" ] || kill "$buildpid" 2>/dev/null || true
    rm -f "$conf"
    systemctl daemon-reload
    systemctl start xochitl
}
trap rollback EXIT HUP INT TERM
if [ ! -s "$hashtab" ] || [ ! -f "$stamp" ] || [ "$(cat "$stamp")" != "$current" ]; then
    buildroot=$root/hashtable-build
    mkdir -p "$buildroot/extensions.d"
    ln -sf "$xovi/extensions.d/qt-resource-rebuilder.so" "$buildroot/extensions.d/qt-resource-rebuilder.so"
    systemctl stop xochitl
    QMLDIFF_HASHTAB_CREATE="$hashtab" QML_DISABLE_DISK_CACHE=1 XOVI_ROOT="$buildroot" \
        LD_PRELOAD="$xovi/xovi.so" /usr/bin/xochitl --system > "$root/settings-activation.log" 2>&1 &
    buildpid=$!
    count=0
    while ! grep -q 'Hashtab saved to' "$root/settings-activation.log"; do
        count=$((count + 1))
        [ "$count" -lt 50 ] || { echo 'UI hashtable timed out; restoring stock interface'; exit 1; }
        kill -0 "$buildpid" 2>/dev/null || exit 1
        sleep 1
    done
    kill "$buildpid"
    wait "$buildpid" || true
    buildpid=
    printf '%s\n' "$current" > "$stamp"
fi
mkdir -p "$(dirname "$conf")"
cat > "$conf" <<EOF
[Service]
Environment="LD_PRELOAD=$xovi/xovi.so"
Environment="XOVI_ROOT=$xovi/services/xochitl.service"
Environment="QML_DISABLE_DISK_CACHE=1"
EOF
systemctl daemon-reload
systemctl restart xochitl
sleep 5
systemctl is-active --quiet xochitl
touch "$root/settings-enabled"
trap - EXIT HUP INT TERM
echo 'Settings overlay enabled; five-finger tap opens it in a notebook.'
