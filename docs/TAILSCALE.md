# Paper Pro private connectivity with Tailscale

The Paper Pro can run Tailscale's official static Linux ARM64 binaries in
userspace mode without a TUN kernel device. This gives SSH a stable private
address while retaining the existing tablet SSH key authentication. A separate
loopback socket on port 2222 uses the vendor's Dropbear service template and
host key: the stock Wi-Fi socket is bound to `wlan0` and cannot accept a local
Tailscale proxy connection. The vendor Wi-Fi and USB sockets remain unchanged. It does not
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
ssh rmpp-wifi '/home/root/smart-remarkable/tailscale/tailscale --socket=/run/smart-remarkable-tailscale.sock serve --bg --tcp=22 tcp://127.0.0.1:2222'
```

Use the assigned Tailscale address in a dedicated SSH alias, preserving the
existing identity file and verifying the same tablet host key. Keep the USB and
Wi-Fi aliases for recovery. Set `REMARKABLE_HOST` to the new alias when installing
the Mac availability service, with `REMARKABLE_FALLBACK_HOSTS=rmpp-wifi` for automatic LAN fallback when reconnecting. A healthy tunnel is retained rather than switched mid-request. The bridge remains on loopback, reached through
the authenticated reverse SSH tunnel. No bridge API is exposed publicly.

## Persistence and recovery

The installer pins and checksum-verifies the official distribution, places the
binaries and private state under `/home/root/smart-remarkable/tailscale`, and
installs dedicated systemd units and target links under `/usr/lib/systemd/system`.
Firmware 3.27 mounts `/etc` as a volatile overlay, so a normal `systemctl enable`
there does not survive a reboot. The daemon unit is a real file on the root
filesystem, allowing systemd to mount `/home` before starting it. The installer
briefly remounts the root filesystem to create these files, restoring its original
read-only state afterward. It
does not change Wi-Fi credentials, DNS, accepted routes, the vendor SSH service,
or notebook files. Re-enabling the service links may be necessary after a
firmware update. Enrollment can be revoked or repeated through the Tailscale
account; do not publish or clone the daemon's private state.

```sh
ssh rmpp-wifi 'systemctl status smart-remarkable-tailscaled.service'
ssh rmpp-wifi 'systemctl status smart-remarkable-ssh.socket'
ssh rmpp-wifi '/home/root/smart-remarkable/tailscale/tailscale --socket=/run/smart-remarkable-tailscale.sock status'
```

To repair only the service installation without downloading the binaries again,
copy `device/tailscaled.service`, `device/tailscale-ssh.socket`, and
`device/install-tailscale-services.sh` into the existing tablet Tailscale directory,
then run `sh /home/root/smart-remarkable/tailscale/install-tailscale-services.sh`
over LAN or USB SSH. This restarts Tailscale, so use a direct connection for the repair.

On Paper Pro firmware 3.27.3.0, a full reboot verified that the daemon and
loopback SSH socket started automatically, the private SSH connection retained
the existing host identity, and the Mac supervisor restored the reverse tunnel
and notebook listener in about a minute. The root filesystem remained read-only.
Target dependencies under the vendor unit directory control startup;
`systemctl is-enabled` may still report `disabled` because no enablement link is
stored in `/etc`. Check the target dependencies and active units instead.

Paper Pro Move installation and compatibility have not yet been tested.

References: [static Linux install](https://tailscale.com/docs/install/linux),
[userspace networking](https://tailscale.com/docs/concepts/userspace-networking),
[TCP Serve](https://tailscale.com/docs/reference/tailscale-cli/serve).
