# Native Windows rewrite plan

> Historical design document: the Rust application now lives at the repository root, and the legacy Python/Qt app and its CI build have been removed. References below to the `native/` layout, Python source files, retaining the old app, and the former build pipeline describe the pre-migration repository, not current instructions. See [README.md](README.md), [BUILDING.md](BUILDING.md), and [benchmarks/WINDOWS_VALIDATION.md](benchmarks/WINDOWS_VALIDATION.md) for current usage and validation status.

## 1. Recommendation and scope

Rewrite the application in **Rust, using Win32 controls, WASAPI audio capture, and WinHTTP networking**. Keep it a portable, single-file Windows application; do not introduce a GUI framework, browser runtime, bundled speech model, or general-purpose asynchronous runtime.

The biggest expected gains come from removing the Python interpreter, PySide6/Qt, and the Python API-client dependency graph—not from choosing C over Rust. A C implementation could produce a smaller executable, but Rust is the recommended balance of resource usage, maintainability, and safety for concurrent audio capture, network parsing, and cancellation. Validate this choice with a small release-build prototype before implementing the full UI.

Primary goals:

1. Substantially reduce idle and active private memory.
2. Substantially reduce the downloadable and unpacked distribution size.
3. Keep recording memory bounded, including long recordings and network failures.
4. Preserve streaming transcription, batch transcription, Windows input compatibility, and the existing settings.

Non-goals for the first release: cross-platform support, local speech recognition, a new visual design, an installer/updater, additional providers, or additional input-device settings.

**Planning assumptions:** Windows 10/11 x64; ordinary user privileges; Internet access and a Mistral API key. Confirm the minimum supported Windows version before implementation. ARM64 can be a separate artifact later. “Offline” currently means recording first and then calling a cloud API, not offline/local inference.

## 2. Research findings from this repository

Reviewed all application Python modules, `requirements.txt`, `dictation_hotkey.spec`, `.github/workflows/build.yml`, `README.md`, and `CLAUDE.md`.

This is source-level analysis, not a measured memory or binary-size comparison. The research environment is Linux, has no installed Mistral Python package, and contains no packaged executable. Windows behavior, actual dependency versions, and the underlying Mistral wire protocol still need validation.

### Current components and native replacements

| Source | Actual responsibility | Proposed replacement |
| --- | --- | --- |
| `main.py` | Session orchestration, sounds, Escape polling, fallback, clipboard-only sessions, output dispatch | Explicit session state machine on the Win32 UI thread |
| `audio.py` | `miniaudio` capture at 16 kHz/mono/PCM16; base64 queue; full-session PCM retained in memory | Event-driven WASAPI, streaming conversion, bounded raw-PCM queues, temporary WAV spool |
| `transcription.py` | Python Mistral SDK; shared asyncio thread; realtime WebSocket; batch API via a blocking call in an executor | Small provider protocol adapter over WinHTTP WebSocket/HTTPS |
| `hotkey.py` | `keyboard` low-level hook, exact modifier matching, key suppression, one-second debounce | Native hotkey registration where possible; minimal `WH_KEYBOARD_LL` hook where suppression is required |
| `typing_output.py` | Clipboard paste or Unicode `SendInput`; RDP-aware scan codes | Win32 clipboard APIs and correctly typed `INPUT` arrays |
| `overlay.py` | Translucent topmost status window, recording animation, click-to-stop | Small nonactivating Win32 popup, GDI painting, recording-only timer |
| `tray.py` | Tray status and menu; primary click records to clipboard | `Shell_NotifyIconW`, native popup menu |
| `settings.py` | API key, hotkeys, language, models, URL, output mode, startup | Native dialog and standard controls, created on demand |
| `config.py` | JSON in `%APPDATA%/dictation_hotkey/`; Startup shortcut through PowerShell | Compatible JSON persistence; `IShellLinkW`/`IPersistFile` for startup |
| `log_buffer.py`, `log_viewer.py` | 2,000-entry log ring; eagerly created live Qt text viewer | Byte-bounded ring; lazy native log dialog |
| `dictation_hotkey.spec` | Windowed PyInstaller single-file bundle, UPX requested | Release-mode native PE executable with embedded resources |
| `.github/workflows/build.yml` | Python 3.12, unpinned dependency install, PyInstaller, release upload | Pinned Rust/MSVC toolchain, tests, native build, size reports, release upload |

### Documentation differs from the implementation

Treat the current source as the parity reference:

- `CLAUDE.md` describes `sounddevice`; `audio.py` actually uses `miniaudio`.
- The notes describe `RegisterHotKey`; `hotkey.py` actually installs a suppressing keyboard hook.
- The default enabled shortcut is **Win+H**, not Win+Y. Copilot support expands to Win+C and Win+Shift+F23; a custom shortcut is also supported.
- The overlay is placed near the primary screen's bottom center, not near the caret.
- Escape stops/submits a recording; it does not discard the audio or undo inserted text, despite the README calling it “cancel.”
- `language` is saved and exposed in Settings but is never passed to either API.
- `base_url` affects realtime only; the batch call uses the SDK's default service URL.
- `proto/`, referenced in `CLAUDE.md`, is not present in this checkout.

### Resource problems to address explicitly

1. **Runtime/deployment dependencies.** Qt and Python are likely the largest fixed costs. The SDK and its transitive dependencies add packaging and import overhead. Measure attribution before assigning exact savings.
2. **Unbounded full-session audio.** `_pcm_chunks` stores every captured byte, even during successful realtime sessions and batch-only recording. PCM alone grows at 32,000 bytes/second: approximately 1.92 MB/minute or 115.2 MB/hour, excluding Python objects.
3. **Audio remains resident after stopping.** `AudioCapture.stop()` closes the device but does not clear `_pcm_chunks`; the recording stays referenced until another start or exit.
4. **Batch conversion creates additional large allocations.** `get_wav_bytes()` joins all PCM, constructs an in-memory WAV, and returns it to the SDK. HTTP-layer copying is not established from this source and must be measured.
5. **Unnecessary base64 round trip.** Capture encodes bytes as base64 strings; `_audio_stream()` decodes them before passing bytes to the SDK. Encode only at the transport boundary if the verified wire protocol requires it.
6. **Unused realtime queue in batch mode.** The queue still fills while only the full-session recording is needed. Its 200 slots are nominally about 20 seconds at 100 ms per chunk, though actual callback sizes must be verified. Overflow silently drops realtime chunks.
7. **Eager and duplicated log UI state.** The log viewer is constructed at startup and receives logs while hidden. Both log entry count and displayed line count are limited, but individual messages are not byte-bounded.
8. **Polling/lifetime overhead.** Python has a 500 ms signal-handler tick, 50 ms Escape polling during recording, and 50 ms empty-audio-queue polling. Native event-driven code can remove these.
9. **Session text grows with duration.** `_session_text` is repeatedly concatenated and retained as last text. Audio is the larger issue, but transcript, log, and network-message budgets also matter.

### Correctness issues not to copy into the rewrite

- Normal stop immediately cancels the realtime coroutine, potentially losing queued audio and final text.
- Batch WAV assembly happens before microphone capture is stopped, leaving a possible final-chunk race.
- A failed realtime session and its replacement batch task share mutable `_running` state and emit untagged completion signals. Late callbacks can affect a newer operation.
- Batch processing leaves `_recording` true; repeated stop triggers can submit again.
- Automatic batch fallback can append a complete transcript after a realtime prefix was already typed, duplicating text.
- An empty clipboard-only session may copy `_last_text` from a previous session.
- Non-BMP characters are passed through `ord()` into a 16-bit `wScan`; Unicode output needs UTF-16 surrogate handling.
- `SendInput` return values are not checked; input failure can be reported as successful dictation.
- No coordinated shutdown path explicitly closes all workers and devices.

These are intentional behavioral fixes, not reasons to preserve faulty behavior for parity.

## 3. Technology selection

| Option | Memory/distribution potential | Trade-offs | Decision |
| --- | --- | --- | --- |
| C + Win32 | Excellent; likely smallest executable | Manual memory ownership, network-parser safety, and concurrent cancellation are harder | Viable if absolute binary size outweighs development/safety costs |
| C++ + Win32 | Excellent with disciplined dependencies; RAII helps handle ownership | Larger language surface; avoid heavy frameworks and networking stacks | Good alternative if maintainers strongly prefer C++ |
| Rust + Win32 | Excellent without GUI/runtime frameworks; some standard-library/serialization code remains | Requires careful Windows FFI and release-size tuning | **Recommended** |
| C# NativeAOT + Win32 | Much smaller than Python/Qt; productive | GC/runtime footprint and AOT/interoperability constraints; not the strongest fit for minimum size | Not first choice for these goals |
| Qt, Electron, WebView-based UI, general cross-platform GUI stacks | Unnecessary dependency/runtime cost for this small Windows-only UI | Easier UI development but weak alignment with the objective | Exclude |

### Proposed dependencies

- Rust standard library; no Tokio unless a demonstrated requirement justifies it.
- Microsoft's `windows` bindings with only necessary feature namespaces. Use typed COM interfaces for WASAPI/Media Foundation and small audited wrappers for raw handles. Evaluate `windows-sys` only if measurements show a worthwhile advantage; generated bindings do not require shipping a Windows SDK runtime.
- `serde`/`serde_json` for configuration and API messages; cap input sizes before parsing.
- A small maintained base64 crate if required by the wire protocol.
- Minimal resource-compilation build tooling for `.rc`, icons, version information, and the manifest.
- WinHTTP/Schannel for HTTPS and WebSockets: no bundled OpenSSL, curl, or independent TLS stack.
- System audio APIs: no PortAudio, Python CFFI, or codec package.

Use normal Rust `std`, not `no_std` or a custom allocator initially. Prefer audited dependencies over hand-written JSON/TLS code to save a few kilobytes. Check licenses and transitive features; commit `Cargo.lock`.

### Early decision gates

Before the full rewrite, build a release-mode vertical slice that opens a microphone, sends one real transcription session through WinHTTP, displays a tray icon, and injects a short test string.

Proceed with Rust if its size/memory is comfortably within the budgets below. If not, inspect linker contributions and loaded modules before changing languages. A C/C++ comparison should use the same Win32/WinHTTP/audio architecture; comparing different stacks would not establish the language overhead.

## 4. Resource budgets and measurement

The following are **provisional acceptance targets, not achieved results or promises**. Ratify them after the Windows baseline and prototype. Use MiB for memory and artifact reporting; distinguish compressed download size from executable size.

| Metric | Initial target |
| --- | --- |
| Total unpacked release payload | At most 5 MiB, ideally one EXE plus small license documentation |
| Compressed download | At most 3 MiB; separately report the directly downloadable EXE size |
| Settled idle private bytes | At most 20 MiB after first-use components have settled |
| Active private bytes | At most 60 MiB during a representative 10-minute session, including stop and batch upload |
| Relative improvement | Aim for at least 75% lower idle private bytes and 90% smaller distribution than the packaged Python baseline |
| Long-recording behavior | No audio-duration-proportional RAM growth; explicit transcript/disk limits |
| Idle activity | No application polling timer when inactive; CPU indistinguishable from measurement noise over five minutes |
| Responsiveness | No blocking capture/network/disk work on the UI thread; no material regression in first-text or stop-to-final-text latency |

Native libraries can increase working set on first use, especially TLS and audio conversion. System DLLs are not distributed, but their process memory still counts. Report when a target is missed rather than hiding shared pages or flushing the working set.

### Reproducible baseline procedure

1. Build the current PyInstaller release on Windows and record resolved package versions, Python version, OS build, architecture, and artifact hashes. Existing requirements are not locked.
2. Record EXE/download size, extracted on-disk footprint during execution, startup time to tray readiness, process-tree memory, threads, handles, and loaded modules. PyInstaller one-file extraction and any helper process must be included.
3. Use Process Explorer/VMMap for attribution and Windows performance counters or a PowerShell sampling script for time series. Report **private bytes/private commit**, **private working set**, total working set, CPU, peak memory, thread count, and handle count separately.
4. Compare cold start and warmed idle; settings/logs open and closed; realtime recording; batch recording; stop/finalization; failed connection; 100 consecutive sessions; and a 30–60 minute recording.
5. Include local mock-server tests with reproducible PCM and response timing, plus real API/microphone smoke tests. Hold Windows build, audio device, model, network conditions, and release profile constant.
6. Measure first-use and later-use cases, peak upload memory, and post-session retained memory. Check monotonic growth across repeated sessions; an allocator may retain a stable high-water mark without leaking.
7. Keep benchmark scripts and results in the repository. Use dedicated Windows runs for reliable memory thresholds; shared CI hosts are better for build-size tracking than precise working-set comparisons.

## 5. Native architecture

### Suggested layout

```text
native/
  Cargo.toml
  Cargo.lock
  build.rs
  resources/
    app.rc, app.manifest, tray-idle.ico, tray-recording.ico
  src/
    main.rs
    app.rs                   # state machine, session IDs, event dispatch
    config.rs                # compatible settings, validation, migration
    diagnostics.rs           # bounded/redacted logging and timings
    ui/{window,tray,overlay,settings,logs}.rs
    platform/{handles,hotkeys,input,clipboard,startup}.rs
    audio/{capture,convert,spool}.rs
    service/{winhttp,protocol,realtime,batch}.rs
  tests/
    ... protocol fixtures, state-machine tests, conversion tests
benchmarks/
  ... Windows measurement scripts and result templates
```

Keep the Python implementation runnable during development. Do not delete it until the native build passes feature and resource acceptance tests.

### Ownership and threads

- **UI thread:** hidden controller window, message pump, tray, dialogs, overlay, hotkeys, clipboard, ordered input output, application state. Initialize its COM apartment appropriately for shell operations.
- **Capture worker, session-scoped:** owns WASAPI interfaces in its COM apartment, waits on audio-ready and cancellation events, copies packets into a preallocated bounded queue, promptly releases WASAPI buffers. No network or filesystem I/O in the capture path.
- **Audio-processing/spool worker, session-scoped:** streaming channel/rate/sample conversion, WAV writing, and publication to the realtime send queue. Disk capture must remain independent of network backpressure.
- **Realtime sender and receiver, session-scoped:** blocking WinHTTP operations off the UI thread, one bounded outgoing stream and one incoming event stream. Validate WinHTTP's allowed concurrent send/receive operations, thread-safety, close behavior, and cancellation during the spike. If the chosen model does not satisfy those contracts, use WinHTTP async callbacks behind the same adapter—not a custom socket/TLS implementation.
- **Batch worker:** streams the completed spool file into one HTTPS request after capture finishes. Reuse session worker capacity where simple; do not keep an executor pool resident while idle.

This is an initial active-thread design, not a claim of fewer threads than Python. There should normally be only the UI application thread while idle, apart from OS-library internals. Track actual threads and committed stack memory; correctness is more important than forcing a single worker.

Workers publish typed events into a bounded channel, and notify the controller with `PostMessageW(WM_APP + ...)`. Use coalesced notifications and drain queues promptly; do not create one unbounded Windows message/pointer allocation per audio chunk. Every event carries a **session ID and operation generation**. Reject obsolete events and release their payloads.

Wrap handles, COM objects, hooks, clipboard locks, timers, and temporary files in ownership types. Avoid network or thread teardown inside destructors running on the UI thread. Each operation needs a cancellation signal, deadline, and exactly one terminal result.

### State machine

```text
Idle
  -> Starting
  -> RecordingRealtime | RecordingBatch
RecordingRealtime
  -> RecordingFallback          # transport failure; capture/spool continue
RecordingRealtime + stop
  -> DrainingRealtime           # stop capture, flush PCM, protocol finalization
RecordingBatch/Fallback + stop
  -> SubmittingBatch            # stop capture, finish WAV, upload
DrainingRealtime/SubmittingBatch
  -> Completed | Failed
Completed/Failed
  -> Idle
Any state + quit
  -> ShuttingDown
```

- `Starting` is cancellable and rolls back cleanly if microphone or network initialization fails.
- Separate “microphone capturing” from “transcription processing.” Repeated stop events are idempotent and cannot start another upload.
- Ignore new start requests during draining/submission in the first version; show “Finishing…” rather than overlapping sessions.
- Freeze relevant settings for the duration of a session. Save edits for the next session; apply hotkey changes transactionally.
- Escape retains current stop/submit semantics. Clearly label it **Stop**, not Cancel. A future discard action must be separate and cannot undo text already injected.
- An error followed by a completion event must not erase the error status or finalize a replacement operation.
- Shutdown first disables new work, then cancels operations, releases the microphone, waits for workers with bounded deadlines, deletes spool files, unregisters hooks/hotkeys, and removes the tray icon. Never terminate a thread while it owns shared state.

## 6. Audio and bounded recording storage

### Capture/conversion

1. Use the default Windows capture endpoint through `IMMDeviceEnumerator`, shared-mode `IAudioClient`, and `IAudioCaptureClient`, with event-driven capture.
2. Negotiate/inspect supported formats. **Do not assume devices natively supply 16 kHz mono PCM16.** Common inputs are 44.1/48 kHz, float, and multichannel.
3. Produce exactly 16,000 Hz, mono, signed little-endian 16-bit PCM for the service.
4. Prototype Windows shared-mode automatic conversion where supported and the system Media Foundation audio resampler as the general conversion path. Verify supported formats, conversion latency, working set, and deployment on supported Windows editions, including N editions if those are in scope.
5. If system conversion is unavailable or disproportionately expensive, explicitly choose a small maintained streaming resampler or a capture-only miniaudio build. Record its artifact/memory/license cost before accepting it. Do not silently ship untested nearest-neighbor conversion or assume simple decimation handles every rate.
6. Handle silent packets, discontinuities, clipping, channel layout/downmixing, fractional resampler state, device invalidation, microphone privacy denial, and unplugging. Flush the converter tail on stop.
7. Keep nominal 100 ms network chunks initially (3,200 PCM bytes), independently of the device packet size. Optimize latency only after parity works.

### Buffer policy

- Use reusable PCM slabs/rings; set capacities in **bytes/time**, not just message count.
- Starting budget: roughly two seconds for capture-to-processing buffering and up to five seconds (160,000 PCM bytes) in the realtime send queue. Tune with measured device/network jitter.
- Overflow of the realtime queue makes the realtime stream incomplete: stop that transport and switch to batch fallback while preserving the spool. Do not silently drop audio and continue claiming success.
- Overflow before the spool, disk-full, or conversion failure compromises the recording itself: stop with an actionable error. Do not claim a complete fallback transcript.
- Batch-only mode has no realtime queue, base64 conversion, WebSocket, or warmup transmission.

### Full-session fallback without full-session RAM

The current fallback requires audio from the **beginning** of the session. Therefore spool every session to a temporary WAV file, including healthy realtime sessions.

- Place files under a per-user local application-data temporary directory, not roaming configuration storage. Use unpredictable names, appropriate user-only access, and restrictive sharing.
- Write PCM incrementally. Stop capture, drain processing, flush conversion, and fix the WAV length fields before upload.
- Batch HTTP upload reads from the file in fixed-size blocks. Do not reconstruct a full recording or multipart request in memory.
- Delete the file on completion, failure, or cancellation. Clean up identifiable stale files after a crash, taking single-instance ownership into account.
- Set a documented duration/disk quota based on verified provider limits and WAV size limits; warn and stop cleanly before reaching it. Bounded RAM must not become unbounded disk growth.
- Temporary audio is a privacy trade-off introduced by this design. Do not log paths/content unnecessarily or promise secure erasure. Document retention and crash cleanup. If plaintext local spooling is unacceptable, add encrypted spooling or a strict in-memory duration cap as a separately scoped choice.

At 32 KB/s, disk bandwidth is modest, but capture still must not wait on slow, redirected, scanned, or failing storage.

## 7. Replace the Mistral SDK with a verified protocol adapter

### Protocol discovery is a blocking milestone

`transcription.py` specifies how the SDK is called; it does **not** establish exact URLs, headers, event JSON, audio framing, or finalization messages. Do not infer these from class names or invent a WebSocket protocol.

Before implementing the adapter:

1. Resolve and archive the exact `<2` SDK version used by the baseline build. Inspect its realtime helper and generated batch request code; compare with current official Mistral documentation.
2. Produce a short protocol specification and sanitized fixtures covering:
   - WebSocket endpoint/path/query, model selection, authorization, subprotocols if any;
   - initial session/audio-format configuration and readiness acknowledgments;
   - binary versus JSON/base64 audio frames and message-size limits;
   - text delta, session-created, error, done, and unknown-event messages;
   - end-of-audio/commit semantics, close handshake, ping/keepalive behavior, timeouts;
   - batch endpoint, multipart fields, file metadata, model/language options, response/error JSON;
   - duration/upload-size/rate limits and retry guidance.
3. Record one successful SDK session with synthetic/non-sensitive audio and compare native requests against the same contract. Never save Authorization headers or a user's dictation as fixtures.
4. Verify custom realtime base URL behavior, including URL-path joining. Preserve the fact that existing batch requests use the default endpoint; do not unexpectedly send keys/audio to a custom batch host.

### Realtime behavior

- Preserve defaults: model `voxtral-mini-transcribe-realtime-2602`, base URL `wss://api.mistral.ai`, audio `pcm_s16le` at 16 kHz.
- Preserve warmup initially: **two seconds of silence**, currently twenty 100 ms chunks paced at 50 ms intervals (about one second of wall time). The microphone records concurrently. Do not change it to a two-second sleep or include warmup in the fallback WAV.
- Encode raw PCM once at the wire boundary if required. Reuse scratch buffers.
- Start receiving while sending; assemble fragmented WebSocket messages with a bounded message size and correct UTF-8 handling. Treat close frames as protocol state, not JSON.
- On user stop, stop capture first, drain buffered real audio, send the verified finalization sequence, and wait for final text/done with a deadline. Immediate cancellation is reserved for abort/shutdown.
- Unknown event types are safely ignored with bounded diagnostics; malformed/oversized messages fail the transport predictably.
- Support certificate validation, finite connect/read/write/close timeouts, Windows proxy behavior, and explicit error reporting for authentication, rate limiting, and service failure. Verify WebSocket proxy support on target Windows versions.
- Require secure schemes by default. Local insecure endpoints, if needed for tests, must be explicit development configuration. Do not forward credentials on unexpected cross-host redirects or weaken TLS validation.

### Batch transcription and fallback

- Preserve default batch model `voxtral-mini-latest`; keep model overrides.
- Stream multipart WAV upload through WinHTTP, calculating content length from file size plus multipart framing with checked arithmetic. Confirm SDK-equivalent fields in the protocol milestone.
- Cap response/error body sizes. Parse only required fields; report useful provider errors without secrets.
- Do not automatically retry ambiguous partially sent/processed requests: duplicate billing and duplicate transcription are possible. Offer a deliberate retry where safe.
- On realtime failure before any text output, keep recording and use the existing automatic batch fallback on stop.
- On failure **after a realtime prefix was already injected**, do not append the full batch transcript. Recommended first-release policy: retain/copy the complete batch result, update “Copy Last Text,” and visibly report that automatic insertion was skipped because partial text already exists. Do not attempt to erase/replace arbitrary application text or guess overlap from model wording.
- Clipboard-only sessions have no injected prefix, so they can simply replace their pending transcript with the final complete batch result.
- Preserve `language` in migrated configuration. Enable it only for endpoints verified to support it; otherwise show it as unsupported rather than pretending it affects transcription.

## 8. Win32 UI and integration

### Controller/tray

- A hidden top-level controller window owns the message pump and receives shell notifications. Unlike a message-only window, it can receive the Explorer `TaskbarCreated` broadcast; recreate the tray icon when Explorer restarts.
- Use `NOTIFYICONDATAW`, `Shell_NotifyIconW`, and native menus. Preserve Settings, View Logs, Copy Last Text, batch-mode toggle, and Quit.
- Preserve left-click recording-to-clipboard and recording/idle icons. Show processing separately from microphone-active state.
- Add per-user/session single-instance protection so two copies cannot both capture or inject text. A second launch can ask the existing instance to show Settings.

### Hotkeys

- Try `RegisterHotKey` with `MOD_NOREPEAT` for supported custom combinations. Report registration conflicts; do not assume Windows-reserved combinations are available.
- **Win+H replacement and Copilot suppression are acceptance-critical.** Prototype `WH_KEYBOARD_LL` suppression on Windows 10/11 rather than assuming `RegisterHotKey` alone matches current behavior.
- Keep hook callbacks short: track modifier/key state, enqueue an action, return. No allocation-heavy logging, disk I/O, networking, or transcription work inside the hook.
- Preserve exact-modifier matching and both Copilot combinations. Define custom-key parsing, aliases, layouts, invalid combinations, and duplicate combinations explicitly.
- Suppress matching key repeats and correctly pair suppressed down/up events; use release-based rearming instead of relying only on the current one-second debounce. Validate that holding a key cannot repeatedly toggle recording.
- Tag injected `SendInput` events and ignore the application's own events in the hook. Validate held modifiers, AltGr, left/right Windows keys, and RDP.
- Use the same hook for Escape while recording when available; otherwise an active-session-only Escape check is acceptable. Do not add global Escape suppression unexpectedly.
- If the OS prevents reliable interception of a reserved shortcut, surface that limitation and offer a working custom shortcut rather than claiming parity.

### Output and clipboard

- Preserve `paste` as default, with `shift_insert`, `ctrl_v`, and `ctrl_shift_v`; preserve `keystrokes` mode.
- Use `OpenClipboard`/`EmptyClipboard`/`SetClipboardData(CF_UNICODETEXT)` with correctly transferred `HGLOBAL` ownership and a finite, nonblocking retry policy for clipboard contention.
- Preserve “dictated text remains on clipboard”; do not restore a prior clipboard value and reintroduce delayed-rendering/RDP races.
- Serialize clipboard writes and paste injection. Fast consecutive deltas can overwrite the clipboard before a slow target consumes it; validate and tune bounded coalescing/pacing on RDP and terminals. `SendInput` completion is not proof the target has consumed the clipboard. Avoid arbitrary UI-thread sleeps.
- Use SDK-defined `INPUT` layout, including the full union; verify `sizeof(INPUT)` and `SendInput` return counts.
- Encode Unicode as UTF-16 code units, correctly handling surrogate pairs and batch boundaries. Keep bounded atomic input batches, scan-code mapping, extended-key flags, and reverse-order shortcut releases.
- Handle held physical modifiers conservatively; wait/defer with a bounded deadline rather than leaving keys stuck or sending unintended shortcuts.
- Explain that UIPI prevents an unelevated process from injecting into higher-integrity windows. Do not request administrator rights by default or silently bypass restrictions.
- Keep leading-whitespace trimming limited to the first nonempty transcript output. Separate received text from successfully submitted input so output failures do not look successful.
- Retain current “type into the focused window” behavior; do not steal focus with status UI or forcibly reactivate another application's window.
- Keep last completed text for Copy Last Text; empty sessions must not copy an unrelated prior result automatically. Bound transcript/output queues with documented limits and visible failure instead of silent truncation.

### Overlay, dialogs, sound

- Use a small `WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE` popup, appropriate layered-window transparency, and `WM_MOUSEACTIVATE` handling that permits click-to-stop without activating it.
- Paint with GDI using system fonts and simple shapes. Keep transparency and status semantics; exact emoji rendering and Qt animation curves are not required.
- Use a lightweight timer only while the recording indicator is visible. Stop animation while idle. Preserve auto-hide statuses.
- Preserve bottom-center placement initially, respecting monitor work area, taskbar, per-monitor DPI, and display changes. Active-monitor positioning can be a later UX change.
- Create Settings and Logs only when requested. Use standard edit/combo/check/button controls, tab order, password masking, keyboard navigation, and accessible labels.
- Log ring starting limits: 2,000 entries **and** 256 KiB total, with a per-entry cap. The hidden log dialog must not duplicate/update an ever-growing document; render a bounded snapshot when opened and batch visible updates.
- Do not log API keys, raw audio, or recognized text by default; current first-delta logging should not carry over. Keep timing/error diagnostics and explicit opt-in debug logging.
- Play the Windows Speech On/Off sounds asynchronously if present; absent sound files are nonfatal. No bundled audio assets are needed.

## 9. Configuration, migration, and security

Preserve `%APPDATA%/dictation_hotkey/config.json` and these keys:

```text
api_key, hotkey_copilot, hotkey_win_h, hotkey_custom, language,
typing_mode, paste_shortcut, start_with_windows, offline_mode,
model, offline_model, base_url
```

- Match existing defaults and empty-string/default-model semantics. Load JSON with type checks, reasonable field-size limits, and clear recovery from malformed/unreadable files.
- Retain unknown fields where practical and introduce a schema version without breaking legacy reads. Save atomically via a same-directory temporary file and replacement; preserve the old file on failure.
- Back up configuration before migration. Do not silently change the Python application's Startup shortcut while running an experimental native build.
- Use `SHGetKnownFolderPath` and `IShellLinkW`/`IPersistFile` to create/remove the per-user `Dictation Hotkey.lnk` shortcut. No PowerShell process is needed. Report permission or shortcut failures rather than saving a misleading enabled state.
- First native release can read legacy plaintext `api_key` for compatibility. Prefer an explicit migration to Windows Credential Manager or DPAPI-protected per-user storage once rollback behavior is agreed. Never silently remove the only usable credential from the old version; explain DPAPI portability limitations.
- Mask the key in Settings, redact diagnostics, and minimize in-memory copies. Do not promise perfect erasure of Rust strings or provider/library buffers.
- Use an `asInvoker` manifest with DPI/common-controls settings. Do not require elevation, install services, or grant UIAccess.

## 10. Build and distribution

- Start with `x86_64-pc-windows-msvc` and a Windows SDK; embed icons, version metadata, and manifest in the executable.
- Release profile starting point: `opt-level = "s"`, `lto = "thin"`, `codegen-units = 1`, stripped distributed symbols, and `panic = "abort"`. Compare `s` versus `z` and thin versus full LTO using real binary sizes and audio/network performance.
- Do not allow panics/unwinding across Windows callbacks; expected errors return controlled results. Keep private debug symbols separately for crash diagnosis.
- Choose the CRT linkage explicitly. For a truly portable single EXE, evaluate static CRT linkage and verify imports on a clean machine without developer tools/VC redistributables. Do not assume it works because it runs on a build agent.
- Rely on documented system DLLs only; check actual imports and supported-Windows availability. Lazy-load optional first-use components where this meaningfully reduces idle memory.
- No UPX or self-extracting packer by default. A normal native executable avoids extraction overhead and is easier to diagnose/sign; use ZIP for optional download compression.
- Ship EXE, concise usage/license notices, checksums, and optionally a ZIP. No Python, Qt plugins, `.pyd` modules, bundled TLS library, or model weights.
- Update GitHub Actions to run formatting, Clippy, unit/integration tests, locked release builds, artifact-size reporting, and release publication. Preserve the existing Python job temporarily under a distinct artifact name.
- Pin toolchain/dependencies, inspect the dependency tree, generate license/security inventory, and consider signing when a certificate is available. Document SmartScreen behavior; signing is not a memory optimization.

## 11. Implementation phases and exit criteria

### Phase 0 — Baseline and behavioral contract

Deliverables: Windows baseline report, resolved Python dependencies, feature matrix, reproducible audio/protocol test inputs, clarified minimum OS and resource budgets.

Exit: every current user-visible feature is catalogued; benchmark procedure can be repeated; “offline” and Escape semantics are unambiguous.

### Phase 1 — Risk-first feasibility spikes

Build disposable/small harnesses for:

1. WinHTTP realtime authentication, send/receive, graceful finalization, timeout/cancellation, and streamed batch upload.
2. WASAPI capture and conversion on at least 44.1/48 kHz devices; long-session bounded buffering.
3. Win+H/Copilot interception and suppression; custom hotkeys and conflicts.
4. Clipboard/Unicode output including terminal/RDP behavior.
5. Release executable and idle/active memory with the proposed native dependencies.

Exit: protocol specification/fixtures exist; actual cloud transcription works; each major Windows integration risk is tested; choose conversion and WinHTTP concurrency strategies; confirm Rust budgets or record justified revisions.

### Phase 2 — Native shell and persistence

Implement controller window, tray/menu, single-instance behavior, config load/save, startup shortcut, hotkeys, on-demand settings/logs, overlay, and explicit state model using fake audio/service events.

Exit: existing configuration loads; tray actions and settings work; overlay never steals typing focus; Explorer restart and startup behavior pass; idle memory is measured.

### Phase 3 — Audio pipeline and batch mode

Implement capture/conversion, bounded queues, WAV spool, safe stop/drain, streaming batch upload, and clipboard-only output first.

Exit: full utterances transcribe through the batch API; 30–60 minute mock runs have bounded RAM; device/disk errors and cancellation clean up; WAV bytes/sample counts are validated.

### Phase 4 — Realtime and normal output

Implement verified warmup/protocol, concurrent receive, immediate ordered text output, graceful stop, session-tagged events, and all paste/keystroke modes.

Exit: realtime latency is comparable to baseline; final words are preserved; normal recording works in target applications; Unicode and rapid-stop/restart tests pass.

### Phase 5 — Fallback and failure hardening

Implement safe fallback after partial output, stale-event rejection, processing-state UI, timeouts, bounded logs/transcripts/messages, credential redaction, and orderly shutdown.

Exit: network failure at any phase cannot duplicate automatic input, corrupt a newer session, grow memory without bound, or leave the microphone running after stop/quit.

### Phase 6 — Optimize, package, and cut over

Profile real builds, remove unused dependency features, tune buffers/linker settings, validate clean-machine deployment, update CI/releases/docs, and run the complete parity/resource suite.

Exit: published comparison report, agreed size/memory budgets met (or explicit accepted exceptions), clean Windows smoke test, rollback instructions, and native release artifact. Only then retire Python and update `README.md`/`CLAUDE.md` to the new architecture.

## 12. Verification matrix

### Automated tests

- Hotkey parser: aliases, exact modifiers, duplicates, invalid input, repeat suppression, self-injected events.
- State machine: every normal/error transition, repeated stop, quit while connecting/uploading, old-generation deltas/completions, settings changes mid-session.
- Configuration: defaults, legacy JSON, unknown fields, malformed/oversized input, atomic-save failure, credential migration/rollback.
- Audio: known PCM/float vectors, clipping, silence flags, channel mixing, 44.1/48 kHz conversion, continuity across packet boundaries, resampler tail, WAV header/length correctness.
- Protocol: sanitized SDK-equivalent requests/events, fragmented UTF-8, unknown/malformed/oversized messages, errors, close/finalization, response-size caps, multipart content lengths.
- Output: first-chunk whitespace, empty clipboard session, surrogate pairs, batching boundaries, partial `SendInput`, clipboard contention, output queue limits.
- Storage: network backpressure, queue overflow, disk-full, upload cancellation, stale-file cleanup, recordings reaching the configured limit.
- Leak/stress: repeated fake sessions, stalled consumers, hostile response sizes, every failure injection point. Mock network tests run without API keys; real service tests are explicitly opted in.

### Windows integration/manual acceptance

- Windows 10/11 x64, ordinary user, microphone allowed/denied/missing/unplugged, suspend/resume and default-device changes.
- Win+H, Win+C, Win+Shift+F23, custom shortcut, conflicts, long key hold, additional held modifiers, RDP keyboard layouts.
- Notepad, browser text fields, Office if available, Windows Terminal/PowerShell, classic console, RDP destination, non-BMP text, elevated target restriction.
- Realtime, batch-only, clipboard-only, fallback before first delta, fallback after injected text, no speech, authentication failure, rate limits, slow proxy, lost network, and provider disconnect.
- Stop by hotkey, Escape, overlay, and tray; repeated stop during submission; quit during every state; no final-word loss.
- Settings saved/reloaded, all three paste shortcuts, model/URL overrides, first-run missing key, Startup shortcut, Copy Last Text, log clearing.
- Multi-monitor/DPI changes, accessibility/keyboard navigation, Explorer restart, sound files missing, no overlay focus theft.
- Clean VM with no Python, Rust, Qt, Visual Studio, or manually installed VC runtime; verify artifact imports and runtime dependencies.

### Final acceptance checklist

- [ ] Feature matrix passes, with intentional fixes/limitations documented.
- [ ] Native artifact and all packaged files fit the agreed distribution budget.
- [ ] Idle, active, peak batch-upload, and post-session memory fit the agreed budgets.
- [ ] Long recordings do not retain full-session audio in RAM.
- [ ] Slow network/disk and oversized messages cannot cause unbounded buffering.
- [ ] Final audio/text is drained on normal stop; no stale-session output or duplicated fallback injection.
- [ ] No unexpected privileges, startup changes, credential exposure, or retained temporary recordings.
- [ ] Windows build/test/release instructions and benchmark evidence are checked in.

## 13. Decisions to confirm before implementation

1. Minimum Windows version and whether Windows N editions/ARM64 are supported initially.
2. Acceptance of local temporary WAV spooling, retention limits, and whether encryption is required.
3. Acceptance of the safe partial-fallback policy: copy the full result rather than automatically duplicate/replace already typed text.
4. Whether to relabel “Offline transcription” as “Batch transcription (cloud)” for clarity while retaining the `offline_mode` configuration key.
5. Whether to migrate API keys to protected storage in the first release or a follow-up.
6. Whether currently inactive language settings should remain disabled or become a verified new capability.
7. Final resource budgets after measuring the actual packaged baseline and native spike.

## 14. Implementation reference sources

The repository files above are the evidence for current behavior. The following are starting points for implementation validation, **not wire-protocol facts verified during this source review**:

- Mistral documentation: <https://docs.mistral.ai/>; inspect the audio/realtime and transcription API sections alongside the exact resolved Python SDK source.
- Mistral Python SDK: <https://github.com/mistralai/client-python>.
- Windows Rust bindings: <https://github.com/microsoft/windows-rs>.
- WinHTTP WebSockets: <https://learn.microsoft.com/en-us/windows/win32/winhttp/winhttp-websocket>.
- WASAPI capture: <https://learn.microsoft.com/en-us/windows/win32/coreaudio/capturing-a-stream>.
- System audio resampler: <https://learn.microsoft.com/en-us/windows/win32/medfound/audioresampler>.
- `RegisterHotKey`: <https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registerhotkey>.
- Low-level keyboard hook: <https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc>.
- `SendInput`: <https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput>.

**Bottom line:** replace the heavyweight runtime stack, but also redesign buffer ownership and session lifecycle. A native rewrite that still retains every recording in memory or duplicates transcripts on fallback would miss the main goals despite a smaller EXE.
