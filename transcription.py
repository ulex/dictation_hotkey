import asyncio
import base64
import queue
import threading
import time
import traceback
from typing import AsyncIterator

import log_buffer

_t0: float = 0.0


def _log(msg: str) -> None:
    elapsed = time.perf_counter() - _t0
    log_buffer.log(f"[{elapsed:+.3f}s] {msg}")

from PySide6.QtCore import QObject, Signal

from mistralai import Mistral
from mistralai.extra.realtime import UnknownRealtimeEvent
from mistralai.models import (
    AudioFormat,
    RealtimeTranscriptionError,
    RealtimeTranscriptionSessionCreated,
    TranscriptionStreamDone,
    TranscriptionStreamTextDelta,
)

SAMPLE_RATE = 16_000
WARMUP_DURATION = 2.0  # seconds of silence
DEFAULT_MODEL = "voxtral-mini-transcribe-realtime-2602"
DEFAULT_OFFLINE_MODEL = "voxtral-mini-latest"
DEFAULT_BASE_URL = "wss://api.mistral.ai"

# Shared event loop
_event_loop = None
_loop_thread = None
_loop_lock = threading.Lock()


def _get_event_loop():
    """Get or create the shared asyncio event loop running in a background thread."""
    global _event_loop, _loop_thread
    with _loop_lock:
        if _event_loop is None or not _event_loop.is_running():
            _event_loop = asyncio.new_event_loop()
            _loop_thread = threading.Thread(target=_run_loop, daemon=True)
            _loop_thread.start()
            time.sleep(0.1)
    return _event_loop


def _run_loop():
    asyncio.set_event_loop(_event_loop)
    _event_loop.run_forever()


async def _audio_stream(audio_queue: queue.Queue, is_running: callable) -> AsyncIterator[bytes]:
    """Async generator yielding audio bytes: warmup silence then real mic data."""
    chunk_samples = int(SAMPLE_RATE * 0.1)
    chunk_bytes = b'\x00' * (chunk_samples * 2)  # 2 bytes per int16 sample
    num_chunks = int(WARMUP_DURATION / 0.1)
    for _ in range(num_chunks):
        if not is_running():
            return
        yield chunk_bytes
        await asyncio.sleep(0.05)

    _log("warmup done — real mic audio now streaming")
    while is_running():
        try:
            b64_chunk = audio_queue.get_nowait()
            yield base64.b64decode(b64_chunk)
        except queue.Empty:
            await asyncio.sleep(0.05)


class TranscriptionWorker(QObject):
    """Manages a Mistral realtime transcription session."""

    text_delta = Signal(str)
    status_changed = Signal(str)
    error = Signal(str)
    finished = Signal()

    def __init__(self, parent=None):
        super().__init__(parent)
        self._running = False
        self._task = None

    @property
    def is_running(self) -> bool:
        return self._running

    def start(self, api_key: str, audio_queue: queue.Queue, model: str = "", base_url: str = ""):
        """Start realtime transcription on the shared event loop."""
        global _t0
        _t0 = time.perf_counter()
        _log("start() called — scheduling coroutine")
        self._running = True
        loop = _get_event_loop()
        self._task = asyncio.run_coroutine_threadsafe(
            self._handle(api_key, audio_queue, model or DEFAULT_MODEL, base_url or DEFAULT_BASE_URL), loop
        )

    def start_offline(self, api_key: str, wav_bytes: bytes, offline_model: str = ""):
        """Start offline transcription with the full recorded audio."""
        global _t0
        _t0 = time.perf_counter()
        _log(f"start_offline() called — audio size: {len(wav_bytes) / 1024:.1f} KB")
        self._running = True
        loop = _get_event_loop()
        self._task = asyncio.run_coroutine_threadsafe(
            self._handle_offline(api_key, wav_bytes, offline_model or DEFAULT_OFFLINE_MODEL), loop
        )

    def stop(self):
        """Signal the transcription to stop."""
        self._running = False
        if self._task is not None:
            self._task.cancel()
            self._task = None

    async def _handle(self, api_key: str, audio_queue: queue.Queue, model: str, base_url: str):
        """Core realtime transcription coroutine."""
        _first_delta = True
        try:
            _log(f"_handle() started on async thread (model={model}, base_url={base_url})")
            client = Mistral(api_key=api_key, server_url=base_url)
            audio_format = AudioFormat(encoding="pcm_s16le", sample_rate=SAMPLE_RATE)
            stream = _audio_stream(audio_queue, lambda: self._running)

            _log("opening WebSocket (connecting)")
            self.status_changed.emit("connecting")

            async for event in client.audio.realtime.transcribe_stream(
                audio_stream=stream,
                model=model,
                audio_format=audio_format,
            ):
                if not self._running:
                    break

                if isinstance(event, RealtimeTranscriptionSessionCreated):
                    _log("session created (listening) — warmup audio streaming now")
                    self.status_changed.emit("listening")
                elif isinstance(event, TranscriptionStreamTextDelta):
                    if _first_delta:
                        _log(f"first text_delta received: {event.text!r}")
                        _first_delta = False
                    self.text_delta.emit(event.text)
                elif isinstance(event, TranscriptionStreamDone):
                    break
                elif isinstance(event, RealtimeTranscriptionError):
                    msg = str(event.error)
                    _log(f"ERROR RealtimeTranscriptionError: {msg}")
                    self.error.emit(msg)
                    break
                elif isinstance(event, UnknownRealtimeEvent):
                    _log(f"unknown event: {event!r}")
                    continue

        except asyncio.CancelledError:
            pass
        except Exception as e:
            tb = traceback.format_exc()
            _log(f"EXCEPTION in _handle:\n{tb}")
            msg = str(e) if str(e) else type(e).__name__
            if "CancelledError" not in msg:
                self.error.emit(msg)
        finally:
            self._running = False
            self.finished.emit()

    async def _handle_offline(self, api_key: str, wav_bytes: bytes, offline_model: str):
        """Offline transcription coroutine."""
        try:
            _log(f"_handle_offline() started — submitting to {offline_model}")
            client = Mistral(api_key=api_key)
            response = await asyncio.to_thread(
                client.audio.transcriptions.complete,
                model=offline_model,
                file={"file_name": "recording.wav", "content": wav_bytes},
            )
            text = response.text or ""
            _log(f"offline transcription complete: {len(text)} chars")
            if text:
                self.text_delta.emit(text)
        except asyncio.CancelledError:
            pass
        except Exception as e:
            tb = traceback.format_exc()
            _log(f"EXCEPTION in _handle_offline:\n{tb}")
            msg = str(e) if str(e) else type(e).__name__
            self.error.emit(msg)
        finally:
            self._running = False
            self.finished.emit()
