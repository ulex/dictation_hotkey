import base64
import queue

import numpy as np
import sounddevice as sd

SAMPLE_RATE = 16_000
CHANNELS = 1
DTYPE = "int16"
BLOCKSIZE = 1600  # 100ms at 16kHz


class AudioCapture:
    """Captures microphone audio and puts base64-encoded PCM16 chunks into a queue."""

    def __init__(self):
        self._queue: queue.Queue = queue.Queue(maxsize=200)
        self._stream: sd.InputStream | None = None

    @property
    def queue(self) -> queue.Queue:
        return self._queue

    def _callback(self, indata: np.ndarray, frames, time_info, status):
        pcm_bytes = indata[:, 0].tobytes()
        b64 = base64.b64encode(pcm_bytes).decode("ascii")
        try:
            self._queue.put_nowait(b64)
        except queue.Full:
            pass

    def start(self):
        """Open the mic stream."""
        self._queue = queue.Queue(maxsize=200)
        self._stream = sd.InputStream(
            samplerate=SAMPLE_RATE,
            channels=CHANNELS,
            dtype=DTYPE,
            blocksize=BLOCKSIZE,
            callback=self._callback,
        )
        self._stream.start()

    def stop(self):
        """Close the mic stream."""
        if self._stream is not None:
            self._stream.stop()
            self._stream.close()
            self._stream = None
