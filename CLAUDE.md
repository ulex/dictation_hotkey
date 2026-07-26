# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

System tray dictation app for Windows, macOS and Linux. Global hotkey toggles mic recording on/off. While recording, audio streams to a speech-to-text provider and text is typed into the focused window as it arrives (real-time, not buffered).

Providers: Mistral realtime transcription API (all platforms) or, on macOS, the native Speech framework (`SFSpeechRecognizer`, on-device, no API key — default there).

## Running

```
pip install -r requirements.txt
python main.py
```

No tests or linter configured.

## Architecture

```
Hotkey press (Win+Y / Option+Space)
    │
    ▼
main.py App._on_hotkey()  ──toggles──►  App._start_recording() / _stop_recording()
    │                                        │
    │                                        ├─ audio.py AudioCapture  (Mistral provider only)
    │                                        │    miniaudio CaptureDevice → base64 PCM16 → queue.Queue
    │                                        │
    │                                        ├─ transcription.py TranscriptionWorker  (Mistral)
    │                                        │    Reads queue in async generator → Mistral WebSocket
    │                                        │    Emits text_delta Signal per chunk
    │                                        │
    │                                        ├─ speech_macos.py NativeSpeechWorker  (macOS native)
    │                                        │    AVAudioEngine tap → SFSpeechRecognizer streaming
    │                                        │    Diffs partial results → text_delta + backspaces Signals
    │                                        │
    │                                        └─ typing_output.py type_text()/press_backspace()
    │                                             Called on each delta → per-platform key events
    ▼
overlay.py  ── frameless always-on-top status widget
tray.py     ── QSystemTrayIcon with Settings/Quit menu
settings.py ── QDialog for provider, API key, hotkey, language
config.py   ── JSON persistence (per-platform config dir)
sounds.py   ── per-platform start/stop recording sounds
```

## Platform layers

`hotkey.py` and `typing_output.py` are dispatchers that import a per-platform implementation:

- Windows: `hotkey_windows.py` (low-level keyboard hook), `typing_windows.py` (SendInput)
- macOS: `hotkey_macos.py` (Quartz CGEventTap on a background CFRunLoop), `typing_macos.py` (Quartz CGEvent Unicode events)
- Linux: `hotkey_linux.py` (evdev grab of `/dev/input` keyboards + replay through a uinput virtual keyboard, suppressing matching combos; passive fallback without uinput), `typing_linux.py` (Wayland `zwp_virtual_keyboard_manager_v1` via pywayland with a custom XKB keymap for Unicode; falls back to `wtype`/`ydotool`, or `xdotool` on X11)

Both transcription workers expose the same interface: `text_delta`, `status_changed`, `error`, `finished` signals, `start(api_key, audio_queue, language)`, `stop()`, and `needs_api_key` / `needs_audio_queue` class attributes. `NativeSpeechWorker` adds a `backspaces(int)` signal because `SFSpeechRecognizer` revises partial results.

## Threading Model

Three threads matter:
1. **Main thread (Qt)** — event loop, UI, signal/slot dispatch
2. **Hotkey thread** — Windows: `keyboard` hook thread; macOS: background `CFRunLoop` hosting the Quartz event tap. Emits Qt signal on hotkey
3. **Async thread** — `asyncio.run_forever()` hosts the Mistral WebSocket coroutine and audio stream generator (Mistral provider only)

`TranscriptionWorker` lives as a QObject on the main thread. Its `_handle` coroutine runs on the async thread via `run_coroutine_threadsafe`, but emits Qt signals (`text_delta`, `error`, `finished`) which are delivered to the main thread's event loop. `NativeSpeechWorker` callbacks (recognition results, AVAudioEngine tap) arrive on Apple framework threads and likewise cross into the main thread via queued Qt signals.

## Key Constraints

- **Windows**: uses `ctypes.windll.user32` for SendInput typing and Escape polling
- **Win32 INPUT struct**: the union must include MOUSEINPUT (largest member) for correct `sizeof(INPUT)`, otherwise SendInput silently fails
- **macOS**: hotkey (CGEventTap) and typing (CGEventPost) both require Accessibility permission; native provider additionally needs Speech Recognition + Microphone permission. Default hotkey is Option+Space (Win maps to Cmd; Cmd+H would clash with Hide App)
- **macOS key events**: `CGEventKeyboardSetUnicodeString` length is in UTF-16 code units, not Python characters
- Audio format must be PCM16 mono 16kHz (`pcm_s16le`) to match Mistral's expected format
- 2 seconds of silence warmup is sent before real audio to initialize the Mistral session
- `proto/` is a reference Gradio app (not part of this app) — kept for API usage examples only
- **Linux**: hotkey needs read access to `/dev/input/event*` (group `input`) and `/dev/uinput` for suppression (see README). Wayland typing needs compositor support for virtual-keyboard-v1 (wlroots/KWin; not GNOME). Default hotkey is Alt+Space. libwayland takes ownership of fds passed to `keymap()` events — never close them after posting (it closes them after flushing)

## Testing (Linux)

No system Wayland session is needed — `tests/wayland_compositor.py` is a minimal
pywayland-based headless compositor (wl_compositor/wl_shm/xdg_shell/wl_seat/
wl_output/zwp_virtual_keyboard_manager_v1) used by the end-to-end tests:

- `tests/e2e_wayland_typing.py` — types Unicode text + backspaces into a Qt editor through the real `typing_linux` backend
- `tests/e2e_app_smoke.py` — instantiates the full `main.App` on Wayland and verifies a transcription delta gets typed
- `tests/test_hotkey_linux.py` — unit tests for the evdev hotkey logic with synthetic events

pywayland 0.4.18 needs the server-side patches at the top of `tests/wayland_compositor.py`
(dispatcher handle, NewId arg creation, Array marshaling). A Qt client compositor must
send `wl_buffer.release` after attach or Qt's SHM backing store deadlocks.
