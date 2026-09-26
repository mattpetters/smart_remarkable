# Paper Pro + Codex on a Mac

This fork adds an initial handwriting-to-Codex path. The existing tablet app
captures a lassoed selection together with its surrounding visible page, a Python
bridge on the Mac calls its authenticated Codex CLI, and the tablet traces the
answer with its pen tool using compact IBM Plex Mono lettering. The response is ordinary movable, erasable ink.
Codex inference still uses the cloud service associated with the CLI login.
No OpenAI API key is required for this route.

## First iteration

- Native lasso selection, then a four-finger tap.
- A concise answer returned through `draw_answer`, with a line/character budget
  calculated from available space (up to 16 lines and 52 characters per line).
- An AI label and left margin rule distinguish answers from user handwriting.
- An empty checkbox appears when a request starts; a check marks a completed
  answer and a cross marks failure. The marker remains part of the answer ink.
- Consistent 22px monospaced text and 31px line spacing in virtual screen units;
  short answers no longer expand to fill the available space.
- The answer appears line by line below the selection when space permits.
- Extra gestures during capture, inference, or drawing are discarded immediately;
  they do not queue another question. The bridge also rejects concurrent inference.
- Read-only, ephemeral Codex invocations. The visible page supplies conversation
  context; there is no hidden session history shared across notebooks.
- Authenticated HTTP bound to loopback on the Mac, reached through an SSH
  reverse forward bound to loopback on the tablet.
- Global gesture listener: no notebook ID, title, template, or per-notebook setup.
- Optional Mac login service restores the bridge, SSH tunnel, and tablet listener.
- No XOVI installation or firmware changes.
- Paper Pro Move is not yet supported by the device geometry in this fork.

Native editable text (`REMARKABLE_RESPONSE_MODE=text`) is experimental and not
working reliably on firmware 3.27.3.0: selecting the text tool opens an Add text
menu, and the on-screen keyboard confuses upstream's rotation heuristic. It is
not the default. Its virtual keyboard currently supports ASCII characters only.

The upstream selection-menu LLM button is a separate XOVI extension. It has
not been loaded or verified in this iteration. Use a disposable notebook while
testing placement: the current placement heuristic does not check for existing
ink below the question. Answers are persistent notebook edits.

## Prerequisites

- A Paper Pro already in developer mode, with key-based SSH access.
- A working `rmpp-wifi` SSH alias, or `REMARKABLE_HOST` set to another alias.
- Python 3, Docker, and Codex CLI on the Mac. `codex login status` must succeed.
- The Mac must remain awake and connected while requests run.

## Availability requirements

1. The assistant is available in any open notebook. Selection is read from the
   current screen on each trigger; it is never bound to a particular notebook.
2. It stays available without manually starting a development session. The Mac
   supervisor starts at login, repairs dropped SSH connections, restarts a failed
   bridge, and recreates the tablet listener after a tablet reboot. Healthy
   processes are adopted without interrupting an answer already in progress.
3. Eventually, direct cloud fallback must cover a sleeping or absent Mac. That
   backend and its credential routing are not implemented yet. Today the Mac must
   be awake, logged in, reachable on the LAN, and authenticated with Codex.

Recovery checks run every 15 seconds while healthy and every 30 seconds after a
failure; connection timeouts can add delay. This is automatic recovery, not a
guarantee of zero downtime. Failed prompts are never replayed automatically. Retry
the gesture once the connection returns. Keep the same page open while an answer
is pending; page-change detection is still a separate UX requirement.

The global listener is implemented independently of notebooks. Different
templates, zoom, orientation, and selection/placement edge cases still need
physical testing; one successful notebook test does not validate all layouts.

The tested hardware is `ferrari 1.0`, aarch64, firmware `3.27.3.0`, Qt `6.8.2`,
with `/dev/uinput` already provided by the kernel. Verify separately after any
firmware update; screenshot discovery and virtual input depend on vendor internals.

### Verified on hardware, 2026-09-26

The physical lasso + four-finger gesture successfully sent a handwritten question
through the Mac's Codex CLI and drew a readable answer beneath it as pen strokes.
One short-answer run took approximately 9.5 seconds from trigger to completed
render, including 5.9 seconds in the bridge. This is a single measurement, not a
latency guarantee. The first answer was legible but oversized. The newer compact font, status marker,
and page-context changes have automated coverage and a successful synthetic
two-image Codex test; physical readability and the new status behavior still need
feedback on the device. No notebook images are committed.

## Build and run

From the repository:

```sh
scripts/build-paper-pro.sh
scripts/lan-client.sh deploy
# Optional: use an available model explicitly. Otherwise Codex uses its CLI default.
export REMARKABLE_CODEX_MODEL=YOUR_MODEL_ID
scripts/lan-client.sh start
scripts/lan-client.sh status
# Keep it available after Mac login, reconnection, or a tablet restart:
scripts/lan-client.sh install-autostart
```

Write a short question on an open notebook page with blank space beneath it. Select the writing
with reMarkable's lasso tool, then tap the screen with four fingers. Allow the
answer to finish before interacting with the notebook. The checkbox acknowledges
the request while Codex is thinking and remains pending while the answer is being
written. This is persistent status ink, not a transient overlay or animation. A
failed request leaves a crossed box; retry by selecting the question again.
Placement does not yet detect every collision with existing handwriting.

```sh
scripts/lan-client.sh capture   # read-only screenshot diagnostic
scripts/lan-client.sh stop      # also disables automatic recovery until start
scripts/lan-client.sh uninstall-autostart  # stop and remove the login service
```

`install-autostart` creates `~/Library/LaunchAgents/com.smart-remarkable.lan-client.plist`.
It runs after Mac login, independently of Codex desktop or an open terminal. The
tablet unit remains transient; the supervisor recreates it when the tablet returns.
New units also use systemd restart-on-failure. The normal reMarkable UI stays running.
`start` is idempotent and re-enables an installed LaunchAgent. Without an installed
LaunchAgent it starts the components once, without ongoing recovery. If changing
the host, model, or response mode, stop first and install again with the new
environment variables. Keep the checkout at its installed path.

Runtime PIDs, socket, status, logs, and captures live in the gitignored
`tmp/lan-client/` directory. The durable bridge token lives in
`~/Library/Application Support/Smart Remarkable/bridge.token` with permissions 600;
the containing directory has permissions 700. The legacy temporary token is
migrated without rotation so existing requests continue working. Tablet files
live under `/home/root/smart-remarkable/`.

Logs:

```sh
tail -f tmp/lan-client/bridge.log
tail -f tmp/lan-client/supervisor.log
ssh rmpp-wifi 'journalctl -fu smart-remarkable-lan'
```

The bridge logs elapsed time and response mode, not prompts, screenshots, or
tokens. Upstream tablet logs are separate; avoid debug-level logging of private
notebooks. On each trigger, the selected question and surrounding visible-page image are
sent to Codex together, from the same capture before status ink is drawn. The
page can provide earlier notes and AI-labeled replies for follow-up questions.
Zoomed-out or scrolled-off content, other pages, and closed notebooks are not
included. Temporary bridge images are deleted after the request. The standalone
`capture` binary reads the display without creating virtual input devices or
calling an LLM.

## Validation

```sh
python3 -m unittest discover -s bridge -v
bash -n scripts/lan-client.sh scripts/build-paper-pro.sh
docker run --rm --platform linux/arm64 \
  -v "$PWD:/work" -v smart-remarkable-cargo:/usr/local/cargo -w /work \
  rust:1.96-bookworm \
  cargo test --locked --release --lib --target aarch64-unknown-linux-gnu
```

The bridge tests cover authorization, malformed requests, remote-image rejection,
output bounds, timeouts, single-flight inference, and safe CLI argument handling.
They also verify selection/page ordering, no context carried into a subsequent
request, and cleanup of temporary images. Rendering tests check marker/text
separation, checkbox update bounds, and compact short/long answer layouts.
The trigger test sends a burst larger than the event queue and verifies that none
is replayed after completion, while a subsequent idle trigger is accepted.
Service tests cover adopting a running request, reconnecting without restarting
the listener, recovering a rebooted tablet, preserving credentials, stopping
autostart, and credential migration. Recovery checks do not capture the screen, submit questions, or replay previous requests.
The original font-render test used its author's absolute output path; it now
asserts the in-memory bitmap instead.

Initial device testing also exposed two upstream problems: ruled template lines
could be mistaken for an open toolbar, and pixel-by-pixel toolbar detection
decoded the complete PNG repeatedly. The fork now requires substantial icon
content and caches decoded pixels per capture, with regressions for ruled pages
and out-of-range coordinates.

## Next iterations

1. Refine the handwriting-style ink rendering and answer placement; separately
   implement the firmware's native editable-text flow.
2. Refine the visible status marker; add cancellation, collision-aware placement,
   page-change detection, and concise/long-answer controls.
   Full document context, including off-screen writing, requires document-aware
   capture beyond the current visible-page screenshot.
3. Validate the XOVI selection-menu button on the actual firmware.
4. Add adapters for Claude Code, pi, and Hermes with explicit session and
   action permissions. Add a configured direct-cloud fallback for requests made
   while the Mac is asleep or away, including visible backend/error state.
5. Detect Paper Pro Move and implement its display, pen, touch, and layout
   geometry from measurements on that device.
6. Investigate a terminal view: libghostty-vt plus a Qt/e-ink renderer and SSH
   to a persistent Mac-side PTY. This is a separate UI from notebook answers.

References: [upstream](https://github.com/yangg1224/smart_remarkable),
[XOVI](https://github.com/asivery/xovi),
[rm-literm](https://github.com/asivery/rm-literm),
[libghostty-vt](https://github.com/ghostty-org/ghostty/blob/main/include/ghostty/vt.h).
