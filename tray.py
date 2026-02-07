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
    quit_requested = Signal()

    def __init__(self, hotkey: str = "", parent=None):
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

        menu.addSeparator()

        quit_action = QAction("Quit", menu)
        quit_action.triggered.connect(self.quit_requested.emit)
        menu.addAction(quit_action)

        self.setContextMenu(menu)

    def _update_tooltip(self, recording: bool):
        status = "Recording..." if recording else "Idle"
        tip = f"Dictation Hotkey — {status}"
        if self._hotkey:
            tip += f"\nHotkey: {self._hotkey}"
        self.setToolTip(tip)

    def set_recording(self, recording: bool):
        if recording:
            self.setIcon(self._recording_icon)
        else:
            self.setIcon(self._idle_icon)
        self._update_tooltip(recording)
