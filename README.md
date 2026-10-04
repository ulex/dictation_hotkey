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

## Native rewrite

The Windows-native rewrite lives in [`native/`](native/README.md). It replaces the Python/Qt implementation with Win32 UI, WASAPI microphone capture, WinHTTP realtime/batch transcription, bounded temporary WAV spooling, and native clipboard/SendInput output. The old Python files remain in the repository for reference and rollback.

## Getting Started

Download `DictationHotkey.exe` from the [latest release](../../releases/latest) and run it.

### Prerequisites

- Windows 10/11 x64
- A [Mistral API key](https://console.mistral.ai/) with access to the transcription APIs

### Build

See the [GitHub workflow](./.github/workflows/build.yml) or [`native/README.md`](native/README.md). The release build is produced from `native/` with Rust 1.99 and the `x86_64-pc-windows-msvc` target.

**Beware:** most of the code was AI-generated. Review and smoke-test on Windows before relying on a new release.
