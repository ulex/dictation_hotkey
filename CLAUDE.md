# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

Windows-only system tray dictation app. Global hotkey toggles mic recording on/off. While recording, audio streams to Mistral's realtime transcription API and text is typed into the focused window via SendInput as it arrives (real-time, not buffered).

## Running

```
pip install -r requirements.txt
python main.py
```

No tests or linter configured.

## Architecture

```
Hotkey press (Win+Y)
    │
    ▼
main.py App._on_hotkey()  ──toggles──►  App._start_recording() / _stop_recording()
    │                                        │
    │                                        ├─ audio.py AudioCapture
    │                                        │    sounddevice InputStream → base64 PCM16 → queue.Queue
    │                                        │
    │                                        ├─ transcription.py TranscriptionWorker
    │                                        │    Reads queue in async generator → Mistral WebSocket
    │                                        │    Emits text_delta Signal per chunk
    │                                        │
    │                                        └─ typing_output.py type_text()
    │                                             Called on each text_delta → SendInput KEYEVENTF_UNICODE
    ▼
overlay.py  ── frameless always-on-top status widget
tray.py     ── QSystemTrayIcon with Settings/Quit menu
settings.py ── QDialog for API key, hotkey, language
config.py   ── JSON persistence in %APPDATA%/dictation_hotkey/
```

## Threading Model

Three threads matter:
1. **Main thread (Qt)** — event loop, UI, signal/slot dispatch
2. **Hotkey thread** — Win32 `RegisterHotKey` + `GetMessageW` pump, emits Qt signal on hotkey
3. **Async thread** — `asyncio.run_forever()` hosts the Mistral WebSocket coroutine and audio stream generator

`TranscriptionWorker` lives as a QObject on the main thread. Its `_handle` coroutine runs on the async thread via `run_coroutine_threadsafe`, but emits Qt signals (`text_delta`, `error`, `finished`) which are delivered to the main thread's event loop.

## Key Constraints

- **Windows-only**: uses `ctypes.windll.user32` for hotkey registration and SendInput
- **Win32 INPUT struct**: the union must include MOUSEINPUT (largest member) for correct `sizeof(INPUT)`, otherwise SendInput silently fails
- Audio format must be PCM16 mono 16kHz (`pcm_s16le`) to match Mistral's expected format
- 2 seconds of silence warmup is sent before real audio to initialize the Mistral session
- `proto/` is a reference Gradio app (not part of this app) — kept for API usage examples only
