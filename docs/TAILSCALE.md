# Paper Pro private connectivity with Tailscale

The Paper Pro can run Tailscale's official static Linux ARM64 binaries in
userspace mode without a TUN kernel device. This gives SSH a stable private
address while retaining the existing tablet SSH key authentication. It does not
repair a Wi-Fi association failure or make a sleeping Mac available.

## Install and enroll

With working Wi-Fi or USB SSH:

```sh
scripts/install-tablet-tailscale.sh rmpp-wifi
ssh rmpp-wifi '/home/root/smart-remarkable/tailscale/tailscale --socket=/run/smart-remarkable-tailscale.sock up --hostname=remarkable-paper-pro --accept-dns=false --accept-routes=false --timeout=30s'
```

Complete the displayed login in the same tailnet as the Mac. Then forward SSH
inside the tailnet (this is **Serve**, not public Funnel):

```sh
ssh rmpp-wifi '/home/root/smart-remarkable/tailscale/tailscale --socket=/run/smart-remarkable-tailscale.sock serve --bg --tcp=22 tcp://127.0.0.1:22'
```

Use the assigned Tailscale address in a dedicated SSH alias, preserving the
existing identity file and verifying the same tablet host key. Keep the USB and
Wi-Fi aliases for recovery. Set `REMARKABLE_HOST` to the new alias when installing
the Mac availability service, with `REMARKABLE_FALLBACK_HOSTS=rmpp-wifi` for automatic LAN fallback when reconnecting. A healthy tunnel is retained rather than switched mid-request. The bridge remains on loopback, reached through
the authenticated reverse SSH tunnel. No bridge API is exposed publicly.

## Persistence and recovery

The installer pins and checksum-verifies the official distribution, places the
binaries and private state under `/home/root/smart-remarkable/tailscale`, and
installs a dedicated systemd service. It briefly remounts the root filesystem to
create the service links, restoring its original read-only state afterward. It
does not change Wi-Fi credentials, DNS, accepted routes, the vendor SSH service,
or notebook files. Re-enabling the service links may be necessary after a
firmware update. Enrollment can be revoked or repeated through the Tailscale
account; do not publish or clone the daemon's private state.

```sh
ssh rmpp-wifi 'systemctl status smart-remarkable-tailscaled.service'
ssh rmpp-wifi '/home/root/smart-remarkable/tailscale/tailscale --socket=/run/smart-remarkable-tailscale.sock status'
```

Paper Pro Move installation and compatibility have not yet been tested.

References: [static Linux install](https://tailscale.com/docs/install/linux),
[userspace networking](https://tailscale.com/docs/concepts/userspace-networking),
[TCP Serve](https://tailscale.com/docs/reference/tailscale-cli/serve).
