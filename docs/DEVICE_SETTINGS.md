# Device settings, providers, and fallback

The native toolbar adds **AI** to open settings and **Ask** to send the current
lasso selection. Ask displays an ellipsis while processing; both buttons reject
new actions while an answer is running. The panel also opens with a five-finger
tap in an open notebook. Four fingers still sends the selected question. The panel changes the backend
(Codex, Hermes/oMLX, or Claude Code), model priority, reply length (brief, balanced, detailed), and whether the
visible page is included as context. Save applies the preferences to the next
request; Close discards edits. Sending is disabled while the panel is open, and
settings gestures received during an answer are discarded.

**Answer ink** selects Blue (default), Red, Cyan, or Magenta. The choice is saved
on the tablet and applies to status marks, replies, and illustrations. Existing
preferences without a color use blue. Answers still use medium Ballpoint and
restore the previous tool, pen profiles, and toolbar visibility afterward.
This rendering preference stays on the tablet and is not sent to the model.

Preferences live on the tablet at
`/home/root/smart-remarkable/preferences.json`. They contain no credentials.
The loopback-only API on port 8766 accepts only these enumerated preferences;
preference changes require an open panel and the `X-Smart-Remarkable: 1` header.
Opening the panel and requesting a send use separate header-protected endpoints.
A send is reserved immediately, preventing repeated taps from queuing answers. It does not
expose general application config, arbitrary commands, credentials, or file paths.
The panel closes automatically after five minutes if abandoned.

## Provider priority and status

Use Up/Down to order the three providers. The first is primary; with **Fallbacks on**, failures advance through the remaining entries once. Tap a model name to cycle its choices. The Mac's `backends.json` (`models` array) adds choices to the built-in catalog. **First only** disables provider failover. There is one model choice per provider in this version.

Claude includes `sonnet`, `opus`, `fable`, and `haiku`; these aliases track the installed CLI's recommended releases. The catalog also includes explicit IDs for Sonnet 5, Opus 5.5, Fable 5.1, Fable 5, and Haiku 4.5. Opus 5.5 requires Claude Code 2.1.280 or newer. Access depends on the signed-in account. Fable can use usage credits, including in noninteractive requests; it is available as a choice but is not selected by default. See the [Claude model configuration](https://code.claude.com/docs/en/model-config) and [current model catalog](https://platform.claude.com/docs/en/models/overview).

The pending ink checkbox includes the starting provider and selected model (a CLI alias such as `sonnet` may be shown). A transient toolbar banner reports the current provider/model during inference and disappears before ink delivery. If another provider answers, the reply names that fallback; continuation-page headers use the answering provider. Models still share the same page context, drawing format, selected Ballpoint color, and tool restoration.

Claude Code uses the Mac's existing `claude auth login` session. The adapter requires a CLI supporting `--safe-mode`, native image stream input, and structured output. It runs without session persistence, personal customizations, or inherited MCP servers. It does not extract subscription credentials or require an Anthropic API key. This is a Mac-hosted backend, not direct tablet-to-cloud service.

The complete chain has a 360-second generation budget. Multi-provider attempts are bounded to at most 150 seconds and reserve time for later providers. Request IDs and receipts cover the whole chain, so a lost HTTP reply rejoins the same work. Timed-out CLI process groups are killed before failover. A failed attempt that may have run a command, edit, or unrecognized tool stops automatic replay to avoid duplicate external actions. Web-only failures can advance. Exhaustion, connection failures, and potential-action failures produce distinct notices.

## Local inference

Hermes runs on the Mac and uses an oMLX vision model over loopback. The tablet
continues to use the authenticated SSH tunnel. Choose **First only** with Hermes first for local-only inference. **Fallbacks on** explicitly permits the selected question and page context to reach later providers, including cloud models. The default order is Codex, Hermes, Claude.

1. Install Hermes and oMLX on the Mac. Start oMLX with a working vision model.
2. Copy `bridge/backends.example.json` to
   `~/.config/smart-remarkable/backends.json` and set the installed Hermes path and
   exact oMLX model ID. This file is outside the temporary runtime directory.
3. If oMLX requires authentication, `omlx_settings` references its existing
   settings file. The credential is read locally, never sent to the tablet or
   included in this repository. Omit that field for an unauthenticated local endpoint.
4. Install the optional `ddgs` package in Hermes's Python environment for web
   search without another API key. The worker enables Hermes web, terminal, and
   file tools. Web research uses the network even though model inference is local.
5. Reload the bridge while idle and select Hermes in the panel.

Each request uses a fresh, temporary Hermes home with personal memories, context
files, background review, and trajectory saving disabled. Images go directly to
native vision instead of an auxiliary transcription model. The worker receives
both the selected question and optional page image. Answer wrapping, pagination,
status markers, and tool restoration are shared with Codex. Request receipts
also cover Hermes, preventing a transport retry from repeating its tools.

Local model latency depends strongly on the model and prompt. Synthetic image
checks returned all 20 requested items with two installed vision models; cold
runs took approximately 79 and 52 seconds respectively. These are development
measurements, not a speed guarantee or validation of handwriting accuracy. A warm
HTTP integration run completed all 20 items in 27 seconds and an identical retry
returned the cached answer immediately without another Hermes invocation.

## Native panel installation

The QML panel is an optional XOVI/qt-resource-rebuilder extension targeting
English Paper Pro firmware 3.27. It inserts controls into the native toolbar grid and overlays the notebook
interface without writing settings controls as notebook ink. It is separate from the upstream
experimental LLM selection button.

Install the official aarch64 XOVI distribution, including qt-resource-rebuilder,
framebuffer-spy, and xovi-message-broker. The helper enables the two framebuffer
modules from the bundled inactive extensions, then:

```sh
scp device/settings/Settings.qml device/settings/AssistantButton.qml device/settings/settings.qmd rmpp-wifi:/home/root/smart-remarkable/
scp device/settings/activate.sh rmpp-wifi:/home/root/smart-remarkable/settings-ui.sh
# Only while idle, after the current notebook has saved:
ssh rmpp-wifi 'chmod 700 /home/root/smart-remarkable/settings-ui.sh && /home/root/smart-remarkable/settings-ui.sh'
```

Activation builds the firmware's QML hashtable (its first save takes at least
60 seconds), then restarts the notebook
interface with a runtime systemd override. It preserves existing vendor service
configuration. The helper restores the stock interface if activation fails.
The Mac supervisor can restore an enabled panel after reboot, before starting
the listener, only when the notebook executable still matches the verified
firmware fingerprint. A firmware change leaves the stock interface active until
compatibility is checked and the extension is activated again.

Disable the panel and return to the stock notebook UI:

```sh
ssh rmpp-wifi '/home/root/smart-remarkable/settings-ui.sh disable'
```

The settings schema has automated round-trip, provider/model validation, and duplicate-send
coverage. The current build passed 58 Python bridge/supervisor tests, 52 Rust tests, and QML lint with the Qt modules installed. The Rust checks exercise all four answer colors, tool/profile restoration after successful and failed requests, and compatibility with older bridge settings. A synthetic native-image request through Claude Haiku returned a valid answer in 4.2 seconds. The updated provider-order panel still needs a physical touch check after unlocking the tablet. The firmware 3.27.3.0 resource patches loaded successfully on Paper Pro.
Physical toolbar buttons and settings interaction were confirmed on this firmware.
With XOVI loaded, capture uses its registered framebuffer address and row stride
instead of relying on allocator layout. Metadata is cached for the lifetime of
the notebook process, including its start time so reused process IDs are safe. A physical capture verified this path
with the extension loaded; RGB channel order and row padding are normalized.
The live settings API also accepted open/close and rejected a send while the
panel was open. QML was
checked with Qt tooling; panel activation, touch interaction, and notebook
restoration require a fresh live device check for other firmware versions.

References: [Hermes programmatic integration](https://hermes-agent.nousresearch.com/docs/developer-guide/programmatic-integration),
[oMLX](https://github.com/jundot/omlx),
[XOVI](https://github.com/asivery/xovi),
[qt-resource-rebuilder](https://github.com/asivery/rm-xovi-extensions/tree/master/qt-resource-rebuilder).
The firmware 3.27 document-view insertion target follows the target used by
[Gestik](https://github.com/alefaraci/xovi-qmd-extensions/blob/main/3.27/gestik.qmd).
