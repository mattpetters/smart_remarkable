#!/usr/bin/env bash
# Official static ARM64 binaries; no Wi-Fi, routing, DNS, or SSH configuration changes.
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
host=${1:?Usage: install-tablet-tailscale.sh SSH_ALIAS}
version=1.102.4
remote=/home/root/smart-remarkable/tailscale
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
url="https://pkgs.tailscale.com/stable/tailscale_${version}_arm64.tgz"
curl --connect-timeout 10 --max-time 180 --fail --silent --show-error --location "$url" -o "$scratch/archive.tgz"
curl --connect-timeout 10 --max-time 180 --fail --silent --show-error --location "$url.sha256" -o "$scratch/checksum"
actual=$(shasum -a 256 "$scratch/archive.tgz" | cut -d ' ' -f 1)
expected=$(cut -d ' ' -f 1 < "$scratch/checksum" | tr -d '\r\n')
test "$actual" = "$expected"
tar -xzf "$scratch/archive.tgz" -C "$scratch"
ssh -o BatchMode=yes "$host" "test \"\$(uname -m)\" = aarch64 && mkdir -p $remote/incoming && chmod 700 $remote"
scp -O "$scratch/tailscale_${version}_arm64/tailscale" "$scratch/tailscale_${version}_arm64/tailscaled" \
    "$repo/device/tailscaled.service" "$host:$remote/incoming/"
ssh -o BatchMode=yes "$host" 'sh -s' <<'REMOTE'
set -eu
root=/home/root/smart-remarkable/tailscale
unit=/etc/systemd/system/smart-remarkable-tailscaled.service
chmod 700 "$root/incoming/tailscale" "$root/incoming/tailscaled"
mv "$root/incoming/tailscale" "$root/tailscale"
mv "$root/incoming/tailscaled" "$root/tailscaled"
mv "$root/incoming/tailscaled.service" "$root/tailscaled.service"
test ! -e "$unit" || test "$(readlink "$unit")" = "$root/tailscaled.service"
restore_readonly=false
if awk '$2 == "/" && $4 ~ /(^|,)ro(,|$)/ {found=1} END {exit !found}' /proc/mounts; then
    restore_readonly=true
    mount -o remount,rw /
fi
restore() { if "$restore_readonly"; then mount -o remount,ro /; fi; }
trap restore EXIT HUP INT TERM
ln -sfn "$root/tailscaled.service" "$unit"
systemctl daemon-reload
systemctl enable smart-remarkable-tailscaled.service
restore
trap - EXIT HUP INT TERM
systemctl restart smart-remarkable-tailscaled.service
systemctl is-active --quiet smart-remarkable-tailscaled.service
"$root/tailscale" --socket=/run/smart-remarkable-tailscale.sock version
REMOTE
echo 'Installed. Enroll with tailscale up --accept-dns=false --accept-routes=false, then enable a tailnet-only TCP forward to localhost:22.'
