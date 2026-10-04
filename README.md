# Dictation Hotkey

I was impressed by the `mistralai/Voxtral-Mini-Realtime` model and wanted to use it as a dictation app for Windows.
Before downloading, you might want to try the [online demo](https://huggingface.co/spaces/mistralai/Voxtral-Mini-Realtime) provided by Mistral.

![](.github/interface.png)


## Features

- **Real-time transcription** — text appears as you speak, not after you stop
- **Multiple hotkey options** — Win+H (replaces Windows dictation), Copilot key, or a custom shortcut
- **On-screen overlay** — shows recording status; click to stop
- **Escape to stop** — press Esc at any time to stop and submit the recording
- **Batch transcription mode** — records first, then uploads a bounded temporary WAV to Mistral
- **Single-file native exe** — no Python, Qt, installer, or bundled speech model required

![](.github/settings.png)

## Native Rust application

Dictation Hotkey is a Windows-native Rust app. The implementation lives in [`native/`](native/README.md) and uses Win32 UI, WASAPI microphone capture, WinHTTP realtime/batch transcription, bounded temporary WAV spooling, and native clipboard/SendInput output. The legacy Python/Qt app and its build pipeline have been removed; only the native application is built and distributed.

## Getting Started

Download `DictationHotkey.exe` from the [latest release](../../releases/latest) and run it.

### Prerequisites

- Windows 10/11 x64
- A [Mistral API key](https://console.mistral.ai/) with access to the transcription APIs
- An internet connection — both realtime and batch modes use Mistral's cloud APIs, not local speech recognition

### Usage

1. Run `DictationHotkey.exe` and enter your API key in Settings.
2. Select Win+H, the Copilot key, or a custom shortcut, then save your settings.
3. Focus the window where you want text inserted and press the shortcut to start dictating.
4. Press the shortcut again, press Esc, or click the overlay to stop.

Right-click the tray icon for Settings, Logs, Copy Last Text, batch mode, and Quit. Left-click the tray icon to start a clipboard-only recording instead of typing into the focused window.

Settings are stored in `%APPDATA%/dictation_hotkey/config.json`. Existing settings from the Python app remain compatible; the first pre-native configuration is backed up as `config.pre-native.json` when settings are saved.

### Build from source

Install Rust 1.99.0 and MSVC Build Tools with the C++ tools and Windows SDK. From the repository root:

```powershell
cd native
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked --target x86_64-pc-windows-msvc
```

The executable is `native/target/x86_64-pc-windows-msvc/release/dictation-hotkey-native.exe`; rename it to `DictationHotkey.exe` for distribution. Python and Qt are not needed to build or run the app.

See [`native/README.md`](native/README.md) for toolchain installation, cross-building, and opt-in Windows tests. The [GitHub workflow](./.github/workflows/build.yml) builds the native executable, ZIP, and SHA-256 checksums.

### Validation

Build and smoke-test results and remaining manual checks are documented in [`benchmarks/WINDOWS_VALIDATION.md`](benchmarks/WINDOWS_VALIDATION.md). Authenticated transcription, microphone recording, and clean-machine behavior still require validation before broad publication.

**Beware:** most of the code was AI-generated. Review and smoke-test on Windows before relying on a new release.
