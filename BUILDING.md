# Building and validating Dictation Hotkey

Windows 10/11 x64 and macOS 13+ native implementation of Dictation Hotkey. This is the only application implementation in the repository; the legacy Python/Qt app and build pipeline have been removed. Both platforms use the shared Rust session workers, config, protocol and bounded temporary WAV spool. Windows keeps the existing `%APPDATA%/dictation_hotkey/config.json` settings format and uses Win32 UI, WASAPI capture, WinHTTP networking and clipboard/SendInput output. macOS supplies AppKit, AVAudioEngine, URLSession and CGEvent adapters.

## What is implemented

- Single-instance hidden Win32 controller and tray menu.
- Win+H/Copilot low-level hotkey suppression plus custom `RegisterHotKey` shortcuts.
- Click-to-stop overlay, Settings dialog, Logs dialog, Copy Last Text, batch-mode toggle, startup shortcut management.
- Event-driven WASAPI capture to 16 kHz mono PCM16 through the Windows audio engine.
- Incremental temporary WAV spool with a one-hour/size cap and drop-time deletion; no full-session audio buffer in RAM.
- Mistral realtime WebSocket adapter over WinHTTP with SDK-equivalent warmup, audio append, flush/end, bounded receive parsing, and graceful finalization.
- Mistral batch upload over WinHTTP using streamed multipart WAV upload.
- Safe fallback policy: before any inserted realtime text, batch fallback can insert/copy the complete result; after partial insertion, the complete batch result is retained for Copy Last Text but is not automatically inserted to avoid duplicates.
- Clipboard paste and Unicode keystroke output with tagged injected events and bounded pacing.

## Build/test

On Windows, install MSVC Build Tools with the C++ tools and Windows SDK, then Rust 1.99.0 (including Clippy and rustfmt):

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools --exact --source winget --accept-source-agreements --accept-package-agreements --silent --override '--wait --quiet --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended'
winget install --id Rustlang.Rustup --exact --source winget
# Open a new terminal so Cargo is on PATH.
rustup toolchain install 1.99.0 --profile minimal --component rustfmt --component clippy
```

Build and run the non-interactive tests from the repository root:

```powershell
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked --target x86_64-pc-windows-msvc
```

From Linux, portable tests and target checks can run, and an MSVC-linked release can be produced with `cargo-xwin`:

```bash
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo check --locked --target x86_64-pc-windows-msvc
cargo xwin build --release --locked --target x86_64-pc-windows-msvc
```

The release executable is:

```text
target/x86_64-pc-windows-msvc/release/dictation-hotkey-native.exe
```

Package it as `DictationHotkey.exe` for distribution. Python and Qt are not required to build or run it.

`tools/verify_sdk_protocol.py` is an optional development-only check against the pinned Mistral Python SDK. It verifies the sanitized fixtures used by the Rust protocol tests without making network requests; it is not part of the app or CI build.

## macOS build and packaging

Install Xcode Command Line Tools and Rust 1.99.0. Swift is supplied by the Apple tools; no additional Rust dependencies are needed for macOS. Build.rs compiles the native Swift framework adapter only for macOS targets.

```sh
xcode-select --install
rustup toolchain install 1.99.0 --profile minimal --component rustfmt --component clippy
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
python3 tools/package_macos.py
```

The packager builds for the host architecture, assembles a `.app` with the Rust executable and Swift dylib, removes the development library search path, signs and verifies the bundle, and creates an architecture-specific ZIP and checksum. The app requires macOS 13+ (Login Items uses ServiceManagement). Python is only used for packaging, never at runtime.

Use `--target aarch64-apple-darwin` or `--target x86_64-apple-darwin` to select an architecture; install that Rust target first. Cross-building the macOS adapter requires an Apple SDK and Swift toolchain on macOS. Existing output bundles are not overwritten; move them aside before packaging again.

Default signing is ad-hoc, suitable for local builds. Pass `--identity 'Developer ID Application: …'` for distribution signing. Public distribution additionally requires Apple's notarization and stapling workflow; CI artifacts are not notarized. The bundle declares [microphone usage](https://developer.apple.com/documentation/avfaudio/avaudioapplication/requestrecordpermission%28completionhandler%3A%29) and the hardened-runtime audio-input entitlement. Microphone and Accessibility approvals are interactive and are not exercised by CI.

For development, `cargo run --locked` locates the Swift dylib in Cargo's build output and embeds the microphone privacy declaration. For normal use, install the bundle in Applications so its identity, permissions and login item remain stable.

macOS adapters retain the shared 16 kHz mono PCM16 format, bounded queues, session tickets, fallback policy and WAV limits. Multipart upload is staged into a private temporary file and streamed by URLSession, with a maximum disk cost of roughly two bounded WAVs, rather than loading audio into memory. Responses and WebSocket messages are bounded, redirects are disabled, and cancellation closes native network tasks. Unknown config fields and the first pre-native backup remain preserved. Mac shortcut and login fields are separate from Windows settings.

## macOS manual validation

Non-interactive tests do not record audio, upload to Mistral, modify the clipboard, inject text or register a login item. Before release, verify on both Apple Silicon and Intel:

- First-run Settings and menu/overlay behavior; one running instance; unavailable shortcut and settings rollback.
- Microphone approval/denial and missing/disconnected input devices; 16 kHz mono PCM16 conversion.
- Accessibility approval/denial; paste and Unicode insertion (including emoji) into a controlled editor; clipboard-only recording and Copy Last Text.
- Authenticated realtime and batch sessions; custom endpoint; disconnect before/after partial insertion, ensuring no duplicate fallback insertion.
- Shortcut repeats, Escape permission behavior, stopping during startup, quit during recording/upload, and deleted temporary files.
- Explicit Login Item enable/disable, approval, failure rollback, relaunch, and running the package on a clean machine without developer tools.

## Validation status

The portable core has automated tests for config migration, hotkey parsing, bounded queues, session state, protocol parsing/framing, WAV spooling, multipart sizing, and output edge cases. Windows build and smoke-test results are recorded in [`benchmarks/WINDOWS_VALIDATION.md`](benchmarks/WINDOWS_VALIDATION.md), with raw idle measurements under `benchmarks/windows/`.

Opt-in tests (run from the repository root):

```powershell
# Close the app first. Use an unlocked desktop and a foreground terminal.
# This launches windows, changes the clipboard, and injects text into a test EDIT.
cargo test --locked --test windows -- --ignored --test-threads=1

# Optional: exercise the packaged release instead of the debug app shell.
$env:DICTATION_TEST_EXE = (Resolve-Path dist/DictationHotkey.exe).Path
cargo test --locked --test windows app_shell -- --ignored --test-threads=1
Remove-Item Env:DICTATION_TEST_EXE

# Shell-level Win+H regression: requires an English unlocked foreground desktop,
# Win+H enabled, empty API key, and no running app. Opens Start as a control,
# then checks both Win keys/release orders trigger Settings without opening Start.
cargo test --locked --test windows_hotkey -- --ignored --test-threads=1

# Simulated one-hour WAV spool (115.2 MB, deleted after the test).
cargo test --locked --lib -- --ignored --test-threads=1

# Opens the default microphone for two seconds; counts/discards audio, no upload.
cargo test --locked --test windows_io wasapi -- --ignored --nocapture

# Contacts Mistral with an invalid key and synthetic silence; no real audio/key needed.
cargo test --locked --test windows_io winhttp -- --ignored --nocapture

# Launch release, dismiss first-run Settings without saving, sample idle, quit.
powershell -NoProfile -ExecutionPolicy Bypass -File tools/measure-native-idle.ps1
```

Desktop input tests assert that the controlled window is foreground before injecting text. A background agent shell, locked desktop, or elevated foreground application can prevent activation; run them from a foreground PowerShell window rather than bypassing Windows input security.

The Win+H shell-level regression now passes against the patched release (the original build reproduces unwanted Start activation). Successful microphone capture, authenticated realtime/batch transcription, physical-key recording toggles (Win+H/Copilot/custom), Escape/overlay stopping, fallback after partial insertion, target-app/RDP behavior, active-session resource budgets, and clean-machine runtime behavior still need validation before broad publication. The Windows session used for the recorded run had no default microphone capture endpoint and no Mistral API key.
