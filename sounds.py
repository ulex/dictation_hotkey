"""Platform-specific start/stop recording sounds."""

import sys

if sys.platform == "darwin":
    # Keep strong references so sounds are not collected before they finish.
    _active_sounds = []

    def _play(name: str):
        try:
            from AppKit import NSSound

            sound = NSSound.soundNamed_(name)
            if sound is None:
                return
            _active_sounds.append(sound)
            sound.play()
            # Drop finished sounds
            _active_sounds[:] = [s for s in _active_sounds if s.isPlaying()]
        except Exception:
            pass

    def play_start():
        _play("Tink")

    def play_stop():
        _play("Pop")

else:
    import os

    if sys.platform == "win32":
        import winsound

        def _play(filename: str):
            windir = os.environ.get("WINDIR", r"C:\Windows")
            winsound.PlaySound(
                os.path.join(windir, "Media", filename),
                winsound.SND_FILENAME | winsound.SND_ASYNC,
            )

        def play_start():
            _play("Speech On.wav")

        def play_stop():
            _play("Speech Off.wav")

    else:  # Linux: synthesize short beeps and play them via miniaudio
        import math
        import struct
        import threading
        import time

        def _tone(freq: float, ms: int, volume: float = 0.25,
                  rate: int = 44100) -> bytes:
            n = int(rate * ms / 1000)
            fade = max(1, int(rate * 0.005))  # 5ms fade to avoid clicks
            buf = bytearray()
            for i in range(n):
                env = min(i + 1, n - i, fade) / fade
                sample = int(32767 * volume * env * math.sin(2 * math.pi * freq * i / rate))
                buf += struct.pack("<h", sample)
            return bytes(buf)

        def _play(pcm: bytes, ms: int):
            def _run():
                try:
                    import miniaudio

                    def _stream():
                        yield  # primed before device.start()
                        for i in range(0, len(pcm), 4096):
                            yield pcm[i:i + 4096]

                    device = miniaudio.PlaybackDevice(
                        output_format=miniaudio.SampleFormat.SIGNED16,
                        nchannels=1,
                        sample_rate=44100,
                    )
                    gen = _stream()
                    next(gen)
                    device.start(gen)
                    time.sleep(ms / 1000 + 0.05)
                    device.stop()
                    device.close()
                except Exception:
                    pass  # no audio device available — sounds are optional

            threading.Thread(target=_run, daemon=True).start()

        def play_start():
            pcm = _tone(880, 90)
            _play(pcm, 90)

        def play_stop():
            pcm = _tone(520, 120)
            _play(pcm, 120)
