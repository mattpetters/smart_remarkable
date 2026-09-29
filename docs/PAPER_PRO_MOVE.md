# Paper Pro Move integration

The Move port is in progress. SSH, registered screen capture, device detection,
and input geometry have been verified on `chiappa 1.0`, firmware `3.29.0.149`.
The notebook UI uses Qt 6.10.3. Complete handwriting replies, toolbar restoration,
and page continuation have not yet been verified; do not use the Paper Pro's
sidebar coordinates on the Move's portrait toolbar.

Verified device values:

- Registered framebuffer: 960 × 1696 BGRA, 3840 bytes per row.
- Pen: `/dev/input/event2`, axes 0–6760 and 0–11960.
- Touch: `/dev/input/event3`, axes 0–1248 and 0–2208.
- Capture requires XOVI's `framebuffer-spy` and `xovi-message-broker`; the stock
  allocator heuristic is not used on the Move.
- The firmware provides `/dev/uinput`; Paper Pro kernel modules are not loaded.

Multiple tablet services can use `REMARKABLE_INSTANCE` to isolate their SSH
sockets, credentials, logs, processes, and Mac login services. A named instance
must set a distinct `REMARKABLE_BRIDGE_PORT`, such as `8767`. Each tablet still
connects to loopback port 8765 through its own SSH reverse tunnel. The default
Paper Pro instance retains its original configuration and port.

Live calibration must cover portrait and landscape toolbar layouts before the
Move's answer listener is enabled.
