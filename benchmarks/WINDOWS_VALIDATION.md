# Native rewrite: Windows validation

Initial run on 2026-10-04 against rewrite commit `09f71ce` plus test/documentation changes, **before the follow-up Win+H patch below**. The initial native Windows build and local smoke checks pass, but end-to-end speech transcription is not yet validated.

## Environment and installed tools

- Windows 11 Pro x64, version `10.0.26200`, build `26200`.
- Rust/Cargo `1.99.0`, host/target `x86_64-pc-windows-msvc`, with rustfmt and Clippy.
- Visual Studio Build Tools 2022 `17.14.41`; MSVC tools `14.44.35207` (dumpbin `14.44.35229.0`).
- Windows SDK `10.0.26100.0`.
- Rust installed with the official `rustup-init.exe`; Build Tools installed using winget's `Microsoft.VisualStudio.2022.BuildTools` package and the VCTools workload.
- No existing app configuration or Mistral API-key environment variable was present. No API key/configuration was saved during validation.
- The only audio endpoint reported by Windows was `Remote Audio`; no default WASAPI capture endpoint was available.

## Build and packaging

Executed on Windows, not cross-compiled:

```powershell
cd native
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked --target x86_64-pc-windows-msvc
```

All four commands pass after fixing a Windows-only Clippy warning in the integration test's `WNDCLASSW` initializer. No production-code change was needed to compile/link the rewrite.

Initial artifacts (ignored by Git; superseded by the Win+H follow-up build below):

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| `native/dist/DictationHotkey.exe` | 489,472 | `8b539fca613e828c3d9922c5dc05b165a8af312f2fba475f6dad07ef03f834a3` |
| `native/dist/DictationHotkey-win64.zip` | 240,406 | `5e1bcdce3349db0bef00d1ae31afc28511026eb41d52225449c3f07aee0daf6a` |

During initial validation, the original cross-built artifacts under root `dist/` were left unchanged. The initial executable was copied from `native/target/x86_64-pc-windows-msvc/release/dictation-hotkey-native.exe`.

`dumpbin /dependents` reports only Windows system DLLs: kernel32, the core synchronization API set, winhttp, user32, gdi32, shell32, winmm, ntdll, oleaut32, ole32, and bcrypt. There is no direct dependency on `vcruntime140.dll`/`msvcp140.dll`, Python, or Qt. This supports single-file distribution but does not replace a clean-machine test. System DLLs may themselves load other system runtime components; the loaded-module list is retained in the idle metadata.

## Tests and observed behavior

| Check | Result | Scope |
| --- | --- | --- |
| Default unit suite | PASS: 25 tests | Config defaults/migration/atomic save/backup, hotkey parsing/matching, bounded queues/logs, protocol fixtures/fragments, session/fallback state, WAV/header/quota/cleanup, multipart/response limits |
| Pre-cancelled WinHTTP transports | PASS: 1 additional default integration test | Realtime/batch return an error promptly; no successful transcription claimed |
| Simulated one-hour spool | PASS | Writes 115.2 MB incrementally, enforces cap, deletes the temporary WAV; not a real one-hour microphone session |
| Desktop app shell (debug) | PASS | Controller/tray startup; second instance exits and opens Settings; password-style API-key control; Settings closes; Logs opens; coordinated Quit exits successfully |
| Desktop app shell (packaged MSVC release) | PASS | Same shell lifecycle against `native/dist/DictationHotkey.exe` using `DICTATION_TEST_EXE` |
| Unicode clipboard/input | PASS | Win64 `INPUT` is 40 bytes; clipboard preserves emoji, supplementary-plane characters, accents, and CRLF; empty copy leaves prior clipboard text intact; Unicode typing across a batching boundary, Shift+Insert paste, and Ctrl+V paste reach a controlled EDIT |
| WinHTTP realtime authentication | PASS (negative path) | Real Mistral endpoint reached over system TLS; deliberately invalid credential returns HTTP 401 |
| WinHTTP streamed batch request | PASS (negative path) | Upload of synthetic silence reaches Mistral and returns HTTP 401 |
| WASAPI microphone probe | BLOCKED/FAILED | `GetDefaultAudioEndpoint(eCapture, eConsole)` reports `0x80070490`, "Element not found"; capture/conversion/stop behavior could not be exercised |
| Authenticated speech transcription | NOT RUN | No valid API key and no capture endpoint |

Desktop tests initially failed because a background agent invocation could not activate its target while an elevated Windows Defender Performance Tool held the foreground. The test now checks foreground/focus before injecting any text, and temporarily attaches input queues only to activate its own test window. The EDIT control also enables horizontal scrolling so longer text is not clipped by its on-screen width. **Both desktop tests subsequently passed together from a normal, non-elevated foreground PowerShell window.** No foreground-stealing workaround was added to application output.

The auth-rejection probes use a fake credential and synthetic silence only. Microphone probe audio, if available, is counted and discarded, not persisted or uploaded. All launched test app processes were closed after validation.

## Idle measurements

Raw data: [`windows/idle.csv`](windows/idle.csv) and [`windows/idle-metadata.json`](windows/idle-metadata.json). Reproduce with `tools/measure-native-idle.ps1` from the repository root.

Release tray app idle for 30 samples at one-second intervals, first-run Settings dismissed without saving, no recording/network session:

| Metric | Minimum | Maximum | Mean |
| --- | ---: | ---: | ---: |
| Private bytes | 2,080,768 | 2,113,536 | 2,085,274 (~1.99 MiB) |
| Working set bytes | 13,611,008 | 13,615,104 | 13,611,554 (~12.98 MiB) |
| Threads | 4 | 4 | 4 |
| Handles | 166 | 167 | 166.13 |

Cumulative CPU stayed at 0.171875 seconds throughout the sample. Coordinated shutdown succeeded. This is a short idle observation on one Windows session, not a leak test or an active-recording/upload benchmark. No legacy Python baseline was measured, so no quantified improvement claim is made.

## Remaining acceptance checks

Use a Windows session with a working default microphone and a valid Mistral key:

1. Run the WASAPI probe, then dictate real speech in realtime mode. Check first-text latency and preservation of final words on stop.
2. Test Win+H replacement, physical Copilot combinations, and a registered custom hotkey. Parser/matcher unit tests do not establish actual global-hook behavior.
3. Stop via the hotkey, Escape, and clicking the overlay; exercise rapid restart and Quit during active capture/network operations.
4. Test batch mode and tray-left-click clipboard-only recording with authenticated API responses.
5. Force realtime failure before any output and after partial insertion. Verify complete batch results are recoverable without duplicate text. Current fallback coverage is state-machine testing, not a live service/session failure.
6. Validate Ctrl+Shift+V in terminals, target editors/browsers, elevated-target/UIPI recovery, and RDP input behavior. Current successful input tests use a controlled Win32 EDIT.
7. Test Settings save/cancel, shortcut conflicts, and Startup shortcut creation/removal with existing real configuration; current desktop test only opens/closes Settings without saving.
8. Measure active capture/upload and repeated-session memory/handles, compare to the legacy build, and run the executable on a separate clean Windows machine without Rust/Build Tools.

Until those checks pass, treat this as a Windows-buildable rewrite with successful local smoke tests, **not a fully proven replacement for real dictation**.

## Follow-up: Win+H opens Start

The reported Win+H failure exposed a gap in the initial validation: the reserved global shortcut had not been exercised against the Windows shell.

Changes:

- Track left/right modifier transitions from hook events rather than reading `GetAsyncKeyState` inside the hook, before Windows has updated that state.
- After an intercepted Win shortcut, replace the original Win-up with a tagged three-event batch: unassigned VK `0xE8` down/up, then Win-up. Otherwise suppressing H/C/F23 leaves the shell seeing a bare Win tap and opening Start. Tagged replacements bypass the matcher; if injection fails, the original Win-up is forwarded to avoid a stuck modifier.
- Add portable regression tests for both Win keys, release orders, repeat suppression, exact modifier matching, bare Win behavior, and tagged-release recursion avoidance.
- Add opt-in `windows_hotkey` integration coverage. It opens Start using a bare Win tap as a positive control, then checks both Win+H release orders on both Win keys activate the app without Start or a leaked H. With an empty key it expects Settings, so it cannot start recording or upload audio.

Formatting, strict Clippy, all 28 default non-ignored tests (27 unit tests plus the cancellation probe), and the MSVC release build pass. Initial shell-level attempts were blocked while `quser` reported the RDP session disconnected; foreground/screenshot access was unavailable. **After the session reconnected, the before/after test completed against the actual Windows shell:**

- Original executable: **FAIL**, `intercepted Win+H opened Start` ([raw output](windows/hotkey-before.log)).
- Patched release executable: **PASS** ([raw output](windows/hotkey-after.log)). Both left/right Win keys and both release orders activate the app without Start or leaked H, no Win key remains stuck, and a bare Win tap opens Start before and after interception.

The probe uses externally tagged `SendInput` key sequences, including a burst with all four events in one batch. It exercises the real application hook and Windows Start menu, not only the portable matcher. Because this profile has an empty API key, app activation opens Settings; it does not establish successful microphone recording. Reproduce with:

```powershell
cd native
cargo test --locked --test windows_hotkey -- --ignored --test-threads=1
```

Requirements: foreground English Windows desktop, no running native app, Win+H enabled, and empty API key (use an unconfigured Windows profile). This test does not modify configuration.

Both `dist/` and `native/dist/` now contain the patched build:

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| `DictationHotkey.exe` | 490,496 | `7a5fe24ae935016d7ef21bcb3faceccc129888a00712fb13cefeeb94a0ddb1b5` |
| `DictationHotkey-win64.zip` (native package) | 240,810 | `32370655140725f739cf376857f11098113fa9f980907cd1fa4bbbe76634932f` |

The previous root executable is retained at `dist/DictationHotkey-before-hotkey-fix.exe`. The initial idle measurements above apply to the previous executable, not this patched build. Shell-level Win+H validation of the fix passed. Physical-key/actual-recording checks and the remaining speech/API acceptance tests are still pending.
