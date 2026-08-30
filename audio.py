import base64
import io
import queue
import traceback
import wave

import miniaudio
import log_buffer

SAMPLE_RATE = 16_000
CHANNELS = 1
BUFFERSIZE_MSEC = 100


class AudioCapture:
    """Captures microphone audio and puts base64-encoded PCM16 chunks into a queue."""

    def __init__(self):
        self._queue: queue.Queue = queue.Queue(maxsize=200)
        self._device: miniaudio.CaptureDevice | None = None
        self._pcm_chunks: list[bytes] = []

    @property
    def queue(self) -> queue.Queue:
        return self._queue

    def _recorder(self):
        """Generator callback that receives captured audio bytes."""
        _ = yield
        while True:
            data = yield
            self._pcm_chunks.append(data)
            b64 = base64.b64encode(data).decode("ascii")
            try:
                self._queue.put_nowait(b64)
            except queue.Full:
                pass

    def start(self):
        """Open the mic stream."""
        try:
            self._queue = queue.Queue(maxsize=200)
            self._pcm_chunks = []
            self._device = miniaudio.CaptureDevice(
                input_format=miniaudio.SampleFormat.SIGNED16,
                nchannels=CHANNELS,
                sample_rate=SAMPLE_RATE,
                buffersize_msec=BUFFERSIZE_MSEC,
            )
            gen = self._recorder()
            next(gen)
            self._device.start(gen)
        except Exception:
            log_buffer.log(f"EXCEPTION in AudioCapture.start:\n{traceback.format_exc()}")
            raise

    def stop(self):
        """Close the mic stream."""
        if self._device is not None:
            self._device.stop()
            self._device.close()
            self._device = None

    def get_wav_bytes(self) -> bytes:
        """Return all captured audio as a WAV file in memory."""
        buf = io.BytesIO()
        with wave.open(buf, "wb") as w:
            w.setnchannels(CHANNELS)
            w.setsampwidth(2)  # 16-bit
            w.setframerate(SAMPLE_RATE)
            w.writeframes(b"".join(self._pcm_chunks))
        return buf.getvalue()
