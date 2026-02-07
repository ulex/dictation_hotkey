import asyncio
import base64
import queue
import threading
import time
from typing import AsyncIterator

import numpy as np
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
MODEL = "voxtral-mini-transcribe-realtime-2602"
BASE_URL = "wss://api.mistral.ai"

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
    num_samples = int(SAMPLE_RATE * WARMUP_DURATION)
    silence = np.zeros(num_samples, dtype=np.int16)
    chunk_size = int(SAMPLE_RATE * 0.1)
    for i in range(0, num_samples, chunk_size):
        if not is_running():
            return
        yield silence[i:i + chunk_size].tobytes()
        await asyncio.sleep(0.05)

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

    def start(self, api_key: str, audio_queue: queue.Queue):
        """Start transcription on the shared event loop."""
        self._running = True
        loop = _get_event_loop()
        self._task = asyncio.run_coroutine_threadsafe(
            self._handle(api_key, audio_queue), loop
        )

    def stop(self):
        """Signal the transcription to stop."""
        self._running = False
        if self._task is not None:
            self._task.cancel()
            self._task = None

    async def _handle(self, api_key: str, audio_queue: queue.Queue):
        """Core transcription coroutine."""
        try:
            client = Mistral(api_key=api_key, server_url=BASE_URL)
            audio_format = AudioFormat(encoding="pcm_s16le", sample_rate=SAMPLE_RATE)
            stream = _audio_stream(audio_queue, lambda: self._running)

            self.status_changed.emit("connecting")

            async for event in client.audio.realtime.transcribe_stream(
                audio_stream=stream,
                model=MODEL,
                audio_format=audio_format,
            ):
                if not self._running:
                    break

                if isinstance(event, RealtimeTranscriptionSessionCreated):
                    self.status_changed.emit("listening")
                elif isinstance(event, TranscriptionStreamTextDelta):
                    self.text_delta.emit(event.text)
                elif isinstance(event, TranscriptionStreamDone):
                    break
                elif isinstance(event, RealtimeTranscriptionError):
                    self.error.emit(str(event.error))
                    break
                elif isinstance(event, UnknownRealtimeEvent):
                    continue

        except asyncio.CancelledError:
            pass
        except Exception as e:
            msg = str(e) if str(e) else type(e).__name__
            if "CancelledError" not in msg:
                self.error.emit(msg)
        finally:
            self._running = False
            self.finished.emit()
