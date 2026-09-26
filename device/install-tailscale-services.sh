#!/bin/sh
# /etc is a volatile overlay on current Paper Pro firmware. Install only our
# units in the persistent vendor unit directory, preserving the stock SSH units.
set -eu
root=/home/root/smart-remarkable/tailscale
units=/usr/lib/systemd/system
test -x "$root/tailscaled"
test -f "$root/tailscaled.service"
test -f "$root/tailscale-ssh.socket"
test -f "$units/dropbear@.service"
test ! -e "$units/smart-remarkable-ssh@.service" || test "$(readlink "$units/smart-remarkable-ssh@.service")" = dropbear@.service
for spec in smart-remarkable-tailscaled.service smart-remarkable-ssh.socket; do
    if test -e "$units/$spec"; then
        grep -q 'notebook assistant private connectivity\|Tailscale for notebook assistant connectivity' "$units/$spec"
    fi
done
restore_readonly=false
if awk '$2 == "/" && $4 ~ /(^|,)ro(,|$)/ {found=1} END {exit !found}' /proc/mounts; then
    restore_readonly=true
    mount -o remount,rw /
fi
restore() { if "$restore_readonly"; then mount -o remount,ro /; fi; }
trap restore EXIT HUP INT TERM
install_unit() {
    cp "$root/$1" "$units/$2.incoming"
    chmod 644 "$units/$2.incoming"
    mv "$units/$2.incoming" "$units/$2"
    mkdir -p "$units/$3.wants"
    ln -sfn "../$2" "$units/$3.wants/$2"
}
install_unit tailscaled.service smart-remarkable-tailscaled.service multi-user.target
install_unit tailscale-ssh.socket smart-remarkable-ssh.socket sockets.target
ln -sfn dropbear@.service "$units/smart-remarkable-ssh@.service"
# Remove only legacy aliases created by the previous installer.
for legacy in /etc/systemd/system/smart-remarkable-tailscaled.service /run/systemd/system/smart-remarkable-tailscaled.service; do
    if test -L "$legacy" && test "$(readlink "$legacy")" = "$root/tailscaled.service"; then rm "$legacy"; fi
done
sync
restore
trap - EXIT HUP INT TERM
systemctl daemon-reload
systemctl start smart-remarkable-ssh.socket
systemctl restart smart-remarkable-tailscaled.service
systemctl is-active --quiet smart-remarkable-ssh.socket
systemctl is-active --quiet smart-remarkable-tailscaled.service
