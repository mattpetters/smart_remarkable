#!/usr/bin/env bash
# Official static ARM64 binaries; preserves vendor Wi-Fi, DNS, and SSH units.
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
    "$repo/device/tailscaled.service" "$repo/device/tailscale-ssh.socket" \
    "$repo/device/install-tailscale-services.sh" "$host:$remote/incoming/"
ssh -o BatchMode=yes "$host" 'sh -s' <<'REMOTE'
set -eu
root=/home/root/smart-remarkable/tailscale
chmod 700 "$root/incoming/tailscale" "$root/incoming/tailscaled"
for file in tailscale tailscaled tailscaled.service tailscale-ssh.socket install-tailscale-services.sh; do
    mv "$root/incoming/$file" "$root/$file"
done
sh "$root/install-tailscale-services.sh"
"$root/tailscale" --socket=/run/smart-remarkable-tailscale.sock version
REMOTE
echo 'Installed. Enroll with tailscale up --accept-dns=false --accept-routes=false, then enable a tailnet-only TCP forward to localhost:2222.'
