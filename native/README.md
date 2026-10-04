# Dictation Hotkey Native

Windows 10/11 x64 native rewrite of Dictation Hotkey. The native app keeps the existing `%APPDATA%/dictation_hotkey/config.json` settings format while replacing the Python/Qt runtime with Win32 UI, WASAPI capture, WinHTTP networking, bounded session state, and native clipboard/SendInput output.

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

On Windows with Rust 1.99, MSVC Build Tools, and a Windows SDK:

```powershell
cd native
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked --target x86_64-pc-windows-msvc
```

From Linux, portable tests and target checks can run, and an MSVC-linked release can be produced with `cargo-xwin`:

```bash
cd native
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo check --locked --target x86_64-pc-windows-msvc
cargo xwin build --release --locked --target x86_64-pc-windows-msvc
```

The release executable is:

```text
native/target/x86_64-pc-windows-msvc/release/dictation-hotkey-native.exe
```

Package it as `DictationHotkey.exe` for distribution.

## Validation status

The portable core has automated tests for config migration, hotkey parsing, bounded queues, session state, protocol parsing/framing, WAV spooling, multipart sizing, and output edge cases. This repository build can compile/check the Windows code from Linux, but microphone behavior, global hotkey interception, WinHTTP service compatibility, RDP/target-app input behavior, memory budgets, and clean-machine runtime behavior still need Windows smoke/performance runs before broad publication.
