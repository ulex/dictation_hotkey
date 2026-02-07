import ctypes
import signal
import sys
import winsound

from PySide6.QtWidgets import QApplication
from PySide6.QtCore import QObject, QTimer, Slot

VK_ESCAPE = 0x1B

import config
from audio import AudioCapture
from transcription import TranscriptionWorker
from typing_output import type_text
from hotkey import GlobalHotkey
from overlay import OverlayWidget
from tray import TrayIcon
from settings import SettingsDialog


class App(QObject):
    """Central controller wiring hotkey -> audio -> transcription -> typing."""

    def __init__(self):
        super().__init__()
        self._config = config.load()
        self._recording = False
        self._chars_typed = 0

        # Components
        self._audio = AudioCapture()
        self._transcription = TranscriptionWorker()
        self._overlay = OverlayWidget()
        self._tray = TrayIcon(hotkey=self._config.get("hotkey", "Win+Y"))
        self._hotkey = GlobalHotkey(self._config.get("hotkey", "Win+Y"))

        # Escape key polling timer
        self._esc_timer = QTimer(self)
        self._esc_timer.setInterval(50)
        self._esc_timer.timeout.connect(self._poll_escape)

        # Connections
        self._hotkey.triggered.connect(self._on_hotkey)
        self._overlay.clicked.connect(self._on_overlay_clicked)
        self._transcription.text_delta.connect(self._on_text_delta)
        self._transcription.error.connect(self._on_error)
        self._transcription.finished.connect(self._on_transcription_finished)
        self._tray.settings_requested.connect(self._open_settings)
        self._tray.quit_requested.connect(QApplication.quit)

        # Start
        self._hotkey.start()
        self._tray.show()

        # Prompt for API key on first run
        if not self._config.get("api_key"):
            self._open_settings()

    @Slot()
    def _on_hotkey(self):
        if not self._recording:
            self._start_recording()
        else:
            self._stop_recording()

    def _start_recording(self):
        api_key = self._config.get("api_key", "")
        if not api_key:
            self._overlay.show_status("Set API key first", auto_hide_ms=2000)
            self._open_settings()
            return

        self._recording = True
        self._chars_typed = 0
        winsound.PlaySound(r"C:\Windows\Media\Speech On.wav", winsound.SND_FILENAME | winsound.SND_ASYNC)
        self._audio.start()
        self._transcription.start(api_key, self._audio.queue)
        self._tray.set_recording(True)
        self._overlay.show_status("Recording...", recording=True)
        self._esc_timer.start()

    def _stop_recording(self):
        self._esc_timer.stop()
        self._recording = False
        self._audio.stop()
        self._transcription.stop()
        self._tray.set_recording(False)

        if self._chars_typed > 0:
            self._overlay.show_status("Done", auto_hide_ms=1500)
        else:
            self._overlay.show_status("No speech detected", auto_hide_ms=1500)

    @Slot(str)
    def _on_text_delta(self, delta: str):
        self._chars_typed += len(delta)
        type_text(delta)

    @Slot()
    def _on_overlay_clicked(self):
        if self._recording:
            self._stop_recording()

    @Slot()
    def _poll_escape(self):
        if ctypes.windll.user32.GetAsyncKeyState(VK_ESCAPE) & 0x8000:
            if self._recording:
                self._stop_recording()

    @Slot(str)
    def _on_error(self, msg: str):
        self._overlay.show_status("Error", auto_hide_ms=2000)
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
            self._config = dlg.get_config()


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
