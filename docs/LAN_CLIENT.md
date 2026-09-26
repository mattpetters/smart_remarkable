# Paper Pro + Codex on a Mac

This fork adds an initial handwriting-to-Codex path. The existing tablet app
captures a lassoed selection, a Python bridge on the Mac calls its authenticated
Codex CLI, and the tablet traces the answer with its pen tool using a
handwriting-style font. The response is ordinary movable, erasable ink.
Codex inference still uses the cloud service associated with the CLI login.
No OpenAI API key is required for this route.

## First iteration

- Native lasso selection, then a four-finger tap.
- One concise answer, at most eight short lines, returned through `draw_answer`.
- The answer appears line by line below the selection when space permits.
- Read-only, ephemeral Codex invocations; this is currently question answering,
  not a persistent coding-agent session.
- Authenticated HTTP bound to loopback on the Mac, reached through an SSH
  reverse forward bound to loopback on the tablet.
- No XOVI installation, firmware changes, or boot-time service installation.
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

The tested hardware is `ferrari 1.0`, aarch64, firmware `3.27.3.0`, Qt `6.8.2`,
with `/dev/uinput` already provided by the kernel. Verify separately after any
firmware update; screenshot discovery and virtual input depend on vendor internals.

## Build and run

From the repository:

```sh
scripts/build-paper-pro.sh
scripts/lan-client.sh deploy
# Optional: use an available model explicitly. Otherwise Codex uses its CLI default.
export REMARKABLE_CODEX_MODEL=YOUR_MODEL_ID
scripts/lan-client.sh start
scripts/lan-client.sh status
```

Write a short question near the top of a new notebook page. Select the writing
with reMarkable's lasso tool, then tap the screen with four fingers. Allow the
answer to finish before interacting with the notebook. This prototype has no
on-device busy indicator and does not yet detect every kind of placement collision.

```sh
scripts/lan-client.sh capture   # read-only screenshot diagnostic
scripts/lan-client.sh stop      # stop tablet worker, SSH tunnel, and Mac bridge
```

The `start` command does not launch on boot. It creates a transient systemd unit
named `smart-remarkable-lan` and leaves the normal reMarkable UI running. Local
runtime files live under the gitignored `tmp/lan-client/` directory. Tablet
files live under `/home/root/smart-remarkable/`. Keep the runtime directory private;
it includes the bridge bearer token and diagnostic screenshots.

Logs:

```sh
tail -f tmp/lan-client/bridge.log
ssh rmpp-wifi 'journalctl -fu smart-remarkable-lan'
```

The bridge logs elapsed time and response mode, not prompts, screenshots, or
tokens. Upstream tablet logs are separate; avoid debug-level logging of private
notebooks. Screenshots are sent to Codex only on a trigger. The standalone
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
2. Add a visible busy state, cancellation, collision-aware placement, and
   concise/long-answer controls.
3. Validate the XOVI selection-menu button on the actual firmware.
4. Add adapters for Claude Code, pi, and Hermes with explicit session and
   action permissions; keep the existing direct API engines as a separate route.
5. Detect Paper Pro Move and implement its display, pen, touch, and layout
   geometry from measurements on that device.
6. Investigate a terminal view: libghostty-vt plus a Qt/e-ink renderer and SSH
   to a persistent Mac-side PTY. This is a separate UI from notebook answers.

References: [upstream](https://github.com/yangg1224/smart_remarkable),
[XOVI](https://github.com/asivery/xovi),
[rm-literm](https://github.com/asivery/rm-literm),
[libghostty-vt](https://github.com/ghostty-org/ghostty/blob/main/include/ghostty/vt.h).
