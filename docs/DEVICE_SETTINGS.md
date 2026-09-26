# Device settings and local Hermes

The optional native panel opens with a five-finger tap in an open notebook.
Four fingers still sends the selected question. The panel changes the backend
(Codex or Hermes/oMLX), reply length (brief, balanced, detailed), and whether the
visible page is included as context. Save applies the preferences to the next
request; Close discards edits. Sending is disabled while the panel is open, and
settings gestures received during an answer are discarded.

Preferences live on the tablet at
`/home/root/smart-remarkable/preferences.json`. They contain no credentials.
The loopback-only API on port 8766 accepts only these enumerated preferences;
mutations require an open panel and the `X-Smart-Remarkable: 1` header. It does not
expose general application config, arbitrary commands, credentials, or file paths.
The panel closes automatically after five minutes if abandoned.

## Local inference

Hermes runs on the Mac and uses an oMLX vision model over loopback. The tablet
continues to use the authenticated SSH tunnel. Selecting Hermes never silently
falls back to a cloud model if local inference fails. Codex remains the default.

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
English Paper Pro firmware 3.27. It overlays the notebook interface without
writing settings controls as notebook ink. It is separate from the upstream
experimental LLM selection button.

Install the official aarch64 XOVI + qt-resource-rebuilder distribution, then:

```sh
scp device/settings/Settings.qml rmpp-wifi:/home/root/smart-remarkable/Settings.qml
scp device/settings/activate.sh rmpp-wifi:/home/root/smart-remarkable/settings-ui.sh
scp device/settings/settings.qmd rmpp-wifi:/home/root/xovi/exthome/qt-resource-rebuilder/smart-remarkable-settings.qmd
# Only while idle, after the current notebook has saved:
ssh rmpp-wifi 'chmod 700 /home/root/smart-remarkable/settings-ui.sh && /home/root/smart-remarkable/settings-ui.sh'
```

Activation builds the firmware's QML hashtable, then restarts the notebook
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

The settings schema has automated round-trip and validation coverage. QML was
checked with Qt tooling; actual panel activation, touch interaction, and notebook
restoration require a live device check on each supported firmware.

References: [Hermes programmatic integration](https://hermes-agent.nousresearch.com/docs/developer-guide/programmatic-integration),
[oMLX](https://github.com/jundot/omlx),
[XOVI](https://github.com/asivery/xovi),
[qt-resource-rebuilder](https://github.com/asivery/rm-xovi-extensions/tree/master/qt-resource-rebuilder).
The firmware 3.27 document-view insertion target follows the target used by
[Gestik](https://github.com/alefaraci/xovi-qmd-extensions/blob/main/3.27/gestik.qmd).
