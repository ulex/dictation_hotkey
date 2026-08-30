from PySide6.QtCore import Signal
from PySide6.QtGui import QIcon, QPixmap, QPainter, QColor, QBrush, QAction
from PySide6.QtWidgets import QSystemTrayIcon, QMenu


def _make_icon(color: str) -> QIcon:
    """Create a simple coloured circle icon."""
    px = QPixmap(32, 32)
    px.fill(QColor(0, 0, 0, 0))
    p = QPainter(px)
    p.setRenderHint(QPainter.RenderHint.Antialiasing)
    p.setBrush(QBrush(QColor(color)))
    p.setPen(QColor(color).darker(130))
    p.drawEllipse(2, 2, 28, 28)
    p.end()
    return QIcon(px)


class TrayIcon(QSystemTrayIcon):
    settings_requested = Signal()
    logs_requested = Signal()
    offline_mode_toggled = Signal(bool)
    quit_requested = Signal()

    def __init__(self, hotkey: str = "", offline_mode: bool = False, parent=None):
        super().__init__(parent)
        self._hotkey = hotkey
        self._idle_icon = _make_icon("#4A90D9")
        self._recording_icon = _make_icon("#DC2626")
        self.setIcon(self._idle_icon)
        self._update_tooltip(recording=False)

        menu = QMenu()
        settings_action = QAction("Settings...", menu)
        settings_action.triggered.connect(self.settings_requested.emit)
        menu.addAction(settings_action)

        logs_action = QAction("View Logs...", menu)
        logs_action.triggered.connect(self.logs_requested.emit)
        menu.addAction(logs_action)

        menu.addSeparator()

        self._offline_action = QAction("Offline transcription", menu)
        self._offline_action.setCheckable(True)
        self._offline_action.setChecked(offline_mode)
        self._offline_action.triggered.connect(self.offline_mode_toggled)
        menu.addAction(self._offline_action)

        menu.addSeparator()

        quit_action = QAction("Quit", menu)
        quit_action.triggered.connect(self.quit_requested.emit)
        menu.addAction(quit_action)

        self.setContextMenu(menu)

    def _update_tooltip(self, recording: bool):
        status = "Recording..." if recording else "Idle"
        tip = f"Dictation Hotkey — {status}"
        if self._hotkey:
            label = "Hotkeys" if ", " in self._hotkey else "Hotkey"
            tip += f"\n{label}: {self._hotkey}"
        self.setToolTip(tip)

    def update_hotkey(self, hotkey: str):
        self._hotkey = hotkey
        self._update_tooltip(recording=False)

    def set_offline_mode(self, enabled: bool):
        self._offline_action.setChecked(enabled)

    def set_recording(self, recording: bool):
        if recording:
            self.setIcon(self._recording_icon)
        else:
            self.setIcon(self._idle_icon)
        self._update_tooltip(recording)
