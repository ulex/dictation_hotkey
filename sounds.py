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
