# Paper Pro + Codex on a Mac

This fork adds an initial handwriting-to-Codex path. The existing tablet app
captures a lassoed selection together with its surrounding visible page, a Python
bridge on the Mac calls its authenticated Codex CLI, and the tablet traces the
answer with its pen tool using compact IBM Plex Mono lettering. The response is ordinary movable, erasable ink.
Codex inference uses the cloud service associated with the CLI login. An optional
Hermes worker uses a local oMLX vision model; see [device settings](DEVICE_SETTINGS.md).
No OpenAI API key is required for this route.

## First iteration

- Native lasso selection, then a four-finger tap.
- A complete, concise answer returned through `draw_answer`, with line width
  calculated from available space. Replies can span pages (up to 128 model lines),
  without reducing the font size or stopping at the first page's line budget.
- An AI label and left margin rule distinguish answers from user handwriting.
- An empty checkbox appears when a request starts; a check marks a completed
  answer and a cross marks failure. An arrow marks a page whose reply continues
  on the next note page; only the final portion receives a check. The marker
  remains part of the answer ink.
- Consistent 22px monospaced text and 31px line spacing in virtual screen units;
  short answers no longer expand to fill the available space.
- The answer appears line by line below the lowest detected writing, including
  earlier AI replies. It never falls back above the question.
- When space is tight, the marked-answer prompt pans down the continuous page
  with two parallel touch contacts and checks the resulting ink movement. It
  rescans newly revealed writing and prefers room for ten full-size lines. At a
  confirmed stationary boundary, it can use existing clear space for a shorter
  first portion (at least four lines). When the verified page has no room, or
  the answer outgrows that space, it inserts a native note page immediately after
  the current page and continues there. Unverified motion aborts before insertion
  or drawing. Bottom UI chrome is excluded.
- The marked-answer prompt temporarily selects the actual Ballpoint pen type,
  medium width, and red color. It snapshots the toolbar's selected row and
  visibility, plus the original pen and Ballpoint profiles. It also captures the
  selected tool's icon so restoration follows tools that move when a PDF note
  page adds a Text tool to the toolbar. The saved selection
  is restored generically, including a highlighter or lasso, after success or
  a normal request/render/cancellation error. Font size and spacing stay unchanged.
- Extra gestures during capture, inference, or drawing are discarded immediately;
  they do not queue another question. The bridge also rejects concurrent inference.
- Blank paragraph separators in a model response are omitted instead of rejecting
  the answer. Pen selection is idempotent; answer and failure marks reuse the pen
  prepared for the pending marker without reopening its settings.
- Interrupted HTTP requests reuse the same request ID to retrieve or rejoin the
  original Codex run. A transport retry never repeats that run's tools. Drawing
  failures propagate to request status instead of reporting partial ink as success.
- Live web search for explicit lookups, current facts, unfamiliar terms, and
  factual uncertainty. Researched replies include a compact source name/domain.
- Full Codex tool access for this personal-use workflow, with short conversational
  answers as the default. The prompt reserves changes and external actions for
  explicit requests in the selected writing.
- Ephemeral Codex invocations. The visible page supplies conversation context;
  there is no hidden session history shared across notebooks.
- Authenticated HTTP bound to loopback on the Mac, reached through an SSH
  reverse forward bound to loopback on the tablet.
- Global gesture listener: no notebook ID, title, template, or per-notebook setup.
- Optional Mac login service restores the bridge, SSH tunnel, and tablet listener.
- The notebook-answer path does not require XOVI. The optional tappable settings
  overlay and native AI/Ask toolbar buttons use XOVI/qt-resource-rebuilder; a
  five-finger gesture also opens settings.
- Paper Pro Move is not yet supported by the device geometry in this fork.

Native editable text (`REMARKABLE_RESPONSE_MODE=text`) is experimental and not
working reliably on firmware 3.27.3.0: selecting the text tool opens an Add text
menu, and the on-screen keyboard confuses upstream's rotation heuristic. It is
not the default. Its virtual keyboard currently supports ASCII characters only.

The upstream selection-menu LLM button is a separate XOVI extension. It has
not been loaded or verified in this iteration. Placement scans visible ink and
filters simple ruled, dotted, and grid templates. Arbitrary templates, faint ink,
and writing farther down beyond a large blank gap can still defeat this image
heuristic. Answers are persistent notebook edits; use a disposable notebook for
initial layout testing.

Note-page insertion uses the native page overview: identify the current page,
select it, choose **More → Add page after**, verify the new current thumbnail,
and open its canvas. It does not use the floating Add button, which appends to
the document's end on firmware 3.27. The captured question and original visible
page remain the model's input, even after scrolling or insertion. A reply can use
up to 12 page areas, with at most 256 lines after local wrapping. The UI controls currently target English Paper Pro
firmware 3.27; an unfamiliar menu or an unchanged current-page indicator stops the
operation. UI-state reads tolerate a delayed redraw, but insertion and pen strokes
are never blindly replayed. The full new canvas is scanned for clear space before
drawing. Document files are never modified directly.

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
   Reverse-forward ownership is recorded using the tablet's boot ID, session PID,
   and process start time. If a disconnected SSH session keeps the bridge port
   occupied, recovery verifies that identity and the listening socket, checks that
   the bridge is unreachable, then terminates only that recorded session. Unknown
   or healthy port owners are left alone. A live stalled-tunnel test verified
   reconnection without restarting the Mac bridge.
3. Eventually, direct cloud fallback must cover a sleeping or absent Mac. That
   backend and its credential routing are not implemented yet. Today the Mac must
   be awake, logged in, reachable on the LAN, and authenticated with Codex.

Recovery checks run every 15 seconds while healthy and every 30 seconds after a
failure; connection timeouts can add delay. This is automatic recovery, not a
guarantee of zero downtime. Each active request allows up to four HTTP attempts,
with 1/2/4-second backoff and a 240-second total recovery deadline. The bridge
retains up to 32 request receipts for 30 minutes in memory: a request digest and
bounded answer or error, without input images. A duplicate ID joins or retrieves
the original invocation; a missing receipt after a restart causes an explicit
failure instead of running the tools again. Model failures and partially drawn
ink are not replayed automatically. Retry the gesture after checking any partial
answer and restoring the question selection. Keep the same page open while an answer
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
latency guarantee. The first answer was legible but oversized. After correcting
repeated pen-tool taps and blank-line response validation, on-device logs confirmed
a compact ten-line reply completed with visible-page context and status markers in
about 19 seconds, including 7.7 seconds in the bridge. There was no second pen-tool
switch during answer rendering. Typography still needs user feedback across more
page layouts. A synthetic two-image Codex test also verified context use. No
notebook images are committed.

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

Write a short question on an open notebook page. Select the writing
with reMarkable's lasso tool, then tap the screen with four fingers. Allow the
answer to finish before interacting with the notebook. The checkbox acknowledges
the request while Codex is thinking and remains pending while the answer is being
written. This is persistent status ink, not a transient overlay or animation. A
failed request leaves a crossed box when the answer area is still known. A
generation or transport failure before rendering also writes a brief retry notice.
Active Paper Pro requests acquire a kernel wake lock so autosleep cannot suspend
network waits and their timeout clock. Cleanup releases it on success or error;
a ten-minute kernel expiry bounds it if the process crashes. The bridge also
receives a four-second health probe before inference, so a broken forward fails
promptly instead of consuming the model's longer response timeout. This uses the
[Linux userspace wake-lock interface](https://github.com/torvalds/linux/blob/master/kernel/power/wakelock.c).
If navigation fails between pages, the last portion retains its continuation
arrow rather than placing a failure mark at unverified coordinates.
The viewport may scroll down before the checkbox appears. The original question
and visible-page context are captured first; scrolling does not replace those
images. Failed layout verification stops before submission or answer ink, so it
cannot leave a failure checkbox; check the tablet journal in that case. Placement
does not yet detect every collision with existing handwriting.

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

The bridge logs elapsed time, response mode, and completed web-lookup event
counts, not prompts, screenshots, search queries, result URLs, or tokens. Upstream tablet logs are separate; avoid debug-level logging of private
notebooks. On each trigger, the selected question and surrounding visible-page image are
sent to Codex together, from the same capture before status ink is drawn. The
page can provide earlier notes and AI-labeled replies for follow-up questions.
Zoomed-out or scrolled-off content, other pages, and closed notebooks are not
included. Temporary bridge images are deleted after the request. The standalone
`capture` binary reads the display without creating virtual input devices or
calling an LLM.

The Mac bridge explicitly invokes `codex --search exec` with
`--sandbox danger-full-access` and `approval_policy="never"`. It keeps
`--ephemeral` and `--ignore-user-config`, so CLI authentication is reused without
loading the user's normal Codex configuration. This is full local tool access;
the request prompt's focus on Q&A/research is not an operating-system sandbox.
Web searches can add latency. The pending checkbox remains while lookup and
answer generation run. Unclear handwriting still prompts clarification; lack of
an external fact should prompt research before an uncertainty response.

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
output bounds, timeouts, single-flight inference, live-search/full-access CLI
arguments, query-free tool-activity counting, and compact source formatting.
Receipt tests cover concurrent retries sharing one run, lost-response retrieval,
conflicting IDs, expired receipts, and cached failures. Rust transport tests drop
or corrupt a response and verify that recovery reuses its original ID. Pagination
tests draw 65 unique lines across four page areas, once each at full size; failed
pen strokes and page edits are not replayed. Long lines are wrapped without
discarding content, and unsupported output is rejected rather than truncated.
A live synthetic image request performed three web lookups and returned a
three-line answer with official-source attribution within the tablet line limits.
They also verify selection/page ordering, no context carried into a subsequent
request, and cleanup of temporary images. Rendering tests check marker/text
separation, checkbox update bounds, and compact short/long answer layouts.
Append-layout tests cover earlier colored answers, faint ink, simple templates,
full pages, newly revealed content, selection-coordinate translation, stalled
scrolling, and unrelated blank captures. A physical run verified one upward pan of approximately 302 virtual pixels,
then stopped before submission when a second pan could not be verified. That
run exposed overly strict space requirements; regression coverage now includes
stationary-boundary fallback, three-pixel template dots, and clipboard chrome.
A complete scrolled answer still needs physical verification.
Two later failures identified more specific causes: the native scrollbar was
being counted as writing, and motion tracking only sampled the lower half of
the screen. The scan now excludes the narrow scrollbar gutter and registers
handwriting throughout the canvas, including a second pan after the first has
moved the question above mid-screen. Regression tests cover both cases. Native
page-insertion errors now name the failed UI stage for diagnosis.
The trigger test sends a burst larger than the event queue and verifies that none
is replayed after completion, while a subsequent idle trigger is accepted.
Temporary-pen tests cover generic selected rows, hidden toolbars, separate pen
profiles, successful replies, request failures, partial setup rollback, and
ambiguous settings detection, and icon-based restoration after a toolbar row shifts. On-device state checks verified selecting red
Ballpoint and restoring the original profile and active tool after a layout
failure. A subsequent on-device request rendered 14 lines at scale 1.0 and
verified restoring the original tool/profile on the success path as well. `examples/check_answer_pen.rs` provides an on-device
check without drawing ink or calling a model; `--fail` exercises error cleanup.
Run it only when the gesture listener is stopped and the tablet is not being used.
The registered framebuffer path places the normalized pen-menu border two pixels
left of the legacy capture. Detection accepts both positions while requiring a
consistent vertical edge and unambiguous pen, size, and color selections. Tests
cover both layouts, and an on-device round trip with XOVI enabled verified red
Ballpoint selection and restoration of both profiles and the original lasso.
The UI reader targets Paper Pro firmware 3.27; an unrecognized settings panel
aborts setup rather than guessing. Process termination cannot perform UI cleanup.
An on-device insertion check created and opened a native PDF note page immediately
after the source page and restored the lasso after its toolbar position changed.
`examples/check_note_page.rs` performs this check without drawing or calling a
model; it creates one page, so run it only on a test document while the listener
is idle and the tablet is not being used. `examples/check_complete_answer.rs`
accepts a synthetic answer fixture and exercises multi-page pen output, including
continuation markers and saved-tool restoration, on newly inserted note pages.
It creates persistent test pages, so run it only in a test document while the
listener is idle and the tablet is not being used.
On-device validation generated all 20 requested items through the live Codex
adapter, then drew items 1–5 near the first test page's bottom and items 6–20 on
the automatically inserted next page. Both pages retained full-size lettering;
the first had a continuation arrow, the last a check, and the original pen
profiles and lasso were restored. The two temporary pages were removed afterward.
This exercises model-to-pen pagination; the combined physical four-finger gesture
and multi-page overflow sequence still needs routine user testing. The current
suite has 45 Python bridge/service tests and 41 Rust library tests.
Service tests cover adopting a running request, reconnecting without restarting
the listener, recovering a rebooted tablet, preserving credentials, stopping
autostart, credential migration, and port reuse after a bridge reload. Recovery checks do not capture the screen, submit questions, or replay previous requests.
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

## Charts and technical illustrations

Ink answers may include up to two labeled line drawings when a graph, chart,
flowchart, circuit, or simple illustration helps explain the question. Codex and
the optional Hermes adapter accept the same structured format: answer lines,
followed by titles, polylines, and text labels. Each illustration uses a bounded
600 by 360 coordinate canvas. The model is instructed to include axes and units,
calculate numeric curves accurately, identify illustrative data, and attribute
researched data in the explanation.

The tablet validates drawings before making answer strokes, escapes label text,
and renders its own SVG. It accepts no model-supplied SVG, code, resource URLs,
or images in this format. Limits are two illustrations, 64 strokes and 1024
points per illustration, and 24 labels. Each reserves a 440-pixel-high area,
including its title and AI marker. Drawings follow the answer in clear space or
continue on a new native note page. They share red Ballpoint selection, tool
restoration, completion markers, and the existing no-replay delivery policy.

A synthetic RC step-response image was answered by Codex with five text lines
and a labeled curve, including the 63.2% point at one time constant. The request
completed in about 35 seconds and its output was rendered through the same Rust
illustration renderer. This validates model output and layout; physical chart
ink still needs a device check. Hermes can produce the structured format, but
the tested local model violated canvas bounds; local illustrations remain
experimental and invalid output is rejected rather than drawn.

To preview the first illustration in a saved bridge answer without a tablet:

```sh
cargo run --example preview_illustration -- answer.json output.svg output.png
```
