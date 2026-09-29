# Paper Pro Move integration

The portrait Move port targets `chiappa 1.0`, firmware `3.29.0.149`, with Qt
6.10.3. Registered capture, blue Ballpoint answers, original pen profile/tool
restoration, and a 17-line reply spanning two native note pages have passed
hardware checks. The Move has its own toolbar adapter; it never uses the
Paper Pro's sidebar coordinates.

Keep the Move in portrait with its toolbar across the top. Other orientations
are rejected before pen setup. Left/right/bottom toolbar configurations and
PDF continuation have not yet been verified on hardware.

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

For the native settings panel, install `device/settings/settings-move.qmd` as
`/home/root/smart-remarkable/settings.qmd`, alongside `Settings.qml`,
`AssistantButton.qml`, and `activate.sh` (renamed `settings-ui.sh`). Run the
activation helper while idle; it builds a firmware-specific QML hashtable and
restarts xochitl. It adds one compact AI button in the Move's toolbar spacer.
The panel scrolls to fit the smaller screen. Lasso a question and tap **Ask**
in the native selection menu alongside Cut/Copy/Delete. The selection remains
intact until capture, and the existing request latch rejects extra sends while
busy. Four fingers remains an alternative; five fingers or AI opens settings.
The toolbar adapter detects small vertical offsets introduced by toolbar layout
changes, and the compact AI button respects the toolbar's gesture margin.

The Mac supervisor restores the registered-capture/settings extension and
listener after reconnect or tablet reboot, provided the notebook firmware
fingerprint still matches. Firmware changes require compatibility checks and
reactivation. This is a Mac-backed client; it does not provide direct cloud
operation while the Mac is unavailable.

Hardware checks are available as `check_answer_pen` (no ink), `check_note_page`
(one native blank page), and `check_complete_answer` (a supplied 17-25-line
fixture on fresh pages). Run them only in a disposable notebook while the
listener is stopped and nobody is writing.
