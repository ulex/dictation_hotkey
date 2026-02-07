import keyboard

from PySide6.QtCore import QObject, Signal

DEFAULT_HOTKEY = "Ctrl+Shift+F23"


class GlobalHotkey(QObject):
    """Registers a system-wide hotkey and emits `triggered` when pressed."""

    triggered = Signal()

    def __init__(self, combo: str = DEFAULT_HOTKEY, parent=None):
        super().__init__(parent)
        self._combo = combo
        self._registered = False

    def start(self):
        """Register the hotkey via low-level keyboard hook."""
        keyboard.add_hotkey(self._combo, self._on_hotkey, suppress=True)
        self._registered = True

    def _on_hotkey(self):
        self.triggered.emit()

    def stop(self):
        """Unregister the hotkey."""
        if self._registered:
            keyboard.remove_hotkey(self._combo)
            self._registered = False

    def update_combo(self, combo: str):
        """Change the hotkey combo."""
        self._combo = combo
