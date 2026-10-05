# CLAUDE.md

This file provides guidance to coding assistants working in this repository.

## What This Is

Windows-only system tray dictation app implemented in Rust at the repository root (`Cargo.toml`, `src/`, `tests/`, and `resources/`). Global hotkeys toggle microphone recording. Realtime mode streams audio to Mistral and inserts text into the focused window as it arrives; batch mode records first and uploads a temporary WAV. Both modes require internet access and a Mistral API key.

The legacy Python/Qt app and PyInstaller build have been removed. `tools/verify_sdk_protocol.py` is an optional development-only SDK fixture checker, not an application or build dependency.

## Build and Test

On Windows, install Rust 1.99.0 and MSVC Build Tools with the C++ tools and Windows SDK. From the repository root:

```powershell
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked --target x86_64-pc-windows-msvc
```

Run `target/x86_64-pc-windows-msvc/release/dictation-hotkey-native.exe` (relative to the repository root). Distribution names it `DictationHotkey.exe`.

See `BUILDING.md` for cross-building and opt-in Windows integration tests. Interactive tests change clipboard/window state or inject input; do not run them unattended. Validation results and remaining manual checks are in `benchmarks/WINDOWS_VALIDATION.md`.

## Architecture

- `src/main.rs`: single-instance Win32 controller, message loop, tray menu, hotkey dispatch, session coordination, and output dispatch.
- `hotkey.rs`: Win+H/Copilot suppression and hotkey matching; custom shortcuts use `RegisterHotKey`.
- `runtime.rs`: session-scoped capture, spooling, realtime networking, batch fallback, and cancellation workers.
- `audio.rs`: event-driven WASAPI microphone capture.
- `spool.rs`: bounded temporary WAV storage and cleanup.
- `service_ws.rs`, `service.rs`, `network_handle.rs`: WinHTTP realtime WebSocket and streamed batch upload.
- `protocol.rs`, `wire.rs`: bounded provider parsing and request framing.
- `session.rs`, `bounded.rs`: session/operation state and bounded event queues.
- `output.rs`: clipboard paste and Unicode `SendInput` output.
- `overlay.rs`, `settings_ui.rs`, `logs_ui.rs`, `ui_font.rs`: native Win32 UI.
- `config.rs`, `paths.rs`, `startup.rs`: JSON persistence, Windows known-folder paths, and per-user Startup shortcut management.

## Threading and Ownership

The Win32 UI thread owns application/session state and dispatches output. Recording creates session, capture, and (for realtime mode) network workers. Bounded channels carry PCM; a bounded event queue posts results to the controller through Win32 messages. Session/operation tickets reject stale events. Stop and abort flags coordinate shutdown, and session workers join capture/network workers before completion.

## Key Constraints

- The app is Windows-only; portable core tests can run on other platforms.
- Audio must be PCM16 mono 16 kHz (`pcm_s16le`). Realtime setup sends the SDK-equivalent silence warmup before microphone audio; warmup must not enter the fallback WAV.
- Keep queues, protocol parsing, logs, session state, and temporary audio storage bounded. Do not buffer entire recordings in RAM.
- After partial realtime insertion, retain the full batch fallback result for Copy Last Text rather than automatically inserting duplicate text.
- Tag injected input events so hotkey hooks ignore them; preserve Windows-key release suppression behavior.
- Settings remain compatible with `%APPDATA%/dictation_hotkey/config.json`; preserve unknown fields and the pre-native backup. Never log API keys or recorded audio.
- Startup changes must be explicit and support restoring the original shortcut on failure.
- CI in `.github/workflows/build.yml` builds and publishes only the native Rust application.

`PLAN.md` is the historical rewrite design, not current build/run guidance. `PROTOCOL.md` records SDK-derived wire behavior.
