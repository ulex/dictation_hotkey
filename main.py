import signal
import sys

from PySide6.QtWidgets import QApplication
from PySide6.QtCore import QObject, QTimer, Slot

import config
import sounds
from audio import AudioCapture
from transcription import TranscriptionWorker
from typing_output import press_backspace, type_text
from hotkey import GlobalHotkey, is_escape_pressed
from overlay import OverlayWidget
from tray import TrayIcon
from settings import SettingsDialog


def _create_worker(cfg: dict) -> QObject:
    """Create a transcription worker for the configured speech provider."""
    provider = cfg.get("provider", "auto")
    if config.IS_MACOS and provider in ("auto", "native"):
        try:
            from speech_macos import NativeSpeechWorker

            return NativeSpeechWorker()
        except ImportError as e:
            print(f"Native Speech provider unavailable ({e}), using Mistral", file=sys.stderr)
    return TranscriptionWorker()


class App(QObject):
    """Central controller wiring hotkey -> audio -> transcription -> typing."""

    def __init__(self):
        super().__init__()
        self._config = config.load()
        self._recording = False
        self._chars_typed = 0

        # Components
        self._audio = AudioCapture()
        self._transcription = None
        self._set_worker(_create_worker(self._config))
        self._overlay = OverlayWidget()
        combos = config.get_hotkey_combos(self._config)
        self._tray = TrayIcon(hotkey=", ".join(combos))
        self._hotkey = GlobalHotkey(combos=combos)

        # Escape key polling timer
        self._esc_timer = QTimer(self)
        self._esc_timer.setInterval(50)
        self._esc_timer.timeout.connect(self._poll_escape)

        # Connections
        self._hotkey.triggered.connect(self._on_hotkey)
        self._overlay.clicked.connect(self._on_overlay_clicked)
        self._tray.settings_requested.connect(self._open_settings)
        self._tray.quit_requested.connect(QApplication.quit)

        # Start
        self._hotkey.start()
        self._tray.show()

        # Prompt for API key on first run if the active provider needs one
        if self._transcription.needs_api_key and not self._config.get("api_key"):
            self._open_settings()

    def _set_worker(self, worker: QObject):
        """Swap the transcription worker and wire up its signals."""
        if self._transcription is not None:
            self._transcription.deleteLater()
        self._transcription = worker
        worker.text_delta.connect(self._on_text_delta)
        worker.error.connect(self._on_error)
        worker.finished.connect(self._on_transcription_finished)
        if hasattr(worker, "backspaces"):
            worker.backspaces.connect(self._on_backspaces)

    @Slot()
    def _on_hotkey(self):
        if not self._recording:
            self._start_recording()
        else:
            self._stop_recording()

    def _start_recording(self):
        if self._transcription.needs_api_key and not self._config.get("api_key", ""):
            self._overlay.show_status("Set API key first", auto_hide_ms=2000)
            self._open_settings()
            return

        self._recording = True
        self._chars_typed = 0
        sounds.play_start()
        if self._transcription.needs_audio_queue:
            self._audio.start()
        self._transcription.start(
            api_key=self._config.get("api_key", ""),
            audio_queue=self._audio.queue,
            language=self._config.get("language", ""),
        )
        self._tray.set_recording(True)
        self._overlay.show_status("🎙️ Listening...", recording=True)
        self._esc_timer.start()

    def _stop_recording(self):
        self._esc_timer.stop()
        self._recording = False
        if self._transcription.needs_audio_queue:
            self._audio.stop()
        self._transcription.stop()
        self._tray.set_recording(False)
        sounds.play_stop()

        if self._chars_typed > 0:
            self._overlay.show_status("Done", auto_hide_ms=1500)
        else:
            self._overlay.show_status("No speech detected", auto_hide_ms=1500)

    @Slot(str)
    def _on_text_delta(self, delta: str):
        if self._chars_typed == 0:  # strip leading space often present in first chunk
            delta = delta.lstrip()
            if not delta:
                return
        self._chars_typed += len(delta)
        type_text(delta)

    @Slot(int)
    def _on_backspaces(self, count: int):
        # The native provider revises partial results; erase stale characters.
        count = min(count, self._chars_typed)
        if count:
            self._chars_typed -= count
            press_backspace(count)

    @Slot()
    def _on_overlay_clicked(self):
        if self._recording:
            self._stop_recording()

    @Slot()
    def _poll_escape(self):
        if is_escape_pressed():
            if self._recording:
                self._stop_recording()

    @Slot(str)
    def _on_error(self, msg: str):
        self._overlay.show_status("Error", auto_hide_ms=2000)
        print(f"Transcription error: {msg}", file=sys.stderr)
        if self._recording:
            self._recording = False
            self._audio.stop()
            self._tray.set_recording(False)

    @Slot()
    def _on_transcription_finished(self):
        if self._recording:
            self._stop_recording()

    @Slot()
    def _open_settings(self):
        dlg = SettingsDialog(self._config)
        if dlg.exec() == SettingsDialog.DialogCode.Accepted:
            old_combos = config.get_hotkey_combos(self._config)
            old_provider = self._config.get("provider", "auto")
            self._config = dlg.get_config()
            new_combos = config.get_hotkey_combos(self._config)
            if self._config.get("provider", "auto") != old_provider:
                self._set_worker(_create_worker(self._config))
            if new_combos != old_combos:
                self._hotkey.stop()
                self._hotkey.update_combos(new_combos)
                self._hotkey.start()
                self._tray.update_hotkey(", ".join(new_combos))


def main():
    signal.signal(signal.SIGINT, signal.SIG_DFL)
    app = QApplication(sys.argv)
    app.setQuitOnLastWindowClosed(False)
    # Timer lets Python's signal handler run inside Qt's event loop
    tick = QTimer()
    tick.start(500)
    tick.timeout.connect(lambda: None)
    controller = App()
    sys.exit(app.exec())


if __name__ == "__main__":
    main()
