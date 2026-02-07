import keyboard

from PySide6.QtCore import QObject, Signal

DEFAULT_HOTKEY = "Ctrl+Shift+F23"


def _parse_combo(combo: str):
    """Parse 'Ctrl+Shift+F23' into (frozenset of modifier names, key name)."""
    parts = [p.strip().lower() for p in combo.split("+")]
    modifiers = frozenset(p for p in parts if p in ("ctrl", "shift", "alt", "win"))
    keys = [p for p in parts if p not in ("ctrl", "shift", "alt", "win")]
    if len(keys) != 1:
        raise ValueError(f"Expected exactly one non-modifier key, got: {keys}")
    return modifiers, keys[0]


class GlobalHotkey(QObject):
    """Registers a system-wide hotkey and emits `triggered` when pressed."""

    triggered = Signal()

    def __init__(self, combo: str = DEFAULT_HOTKEY, parent=None):
        super().__init__(parent)
        self._combo = combo
        self._modifiers, self._key = _parse_combo(combo)
        self._hook = None

    def start(self):
        """Register the hotkey via low-level keyboard hook."""
        self._modifiers, self._key = _parse_combo(self._combo)
        self._hook = keyboard.hook(self._on_event, suppress=True)

    def _on_event(self, event: keyboard.KeyboardEvent):
        """Intercept every key event; suppress and fire on matching combo, pass others through."""
        if event.name and event.name.lower() == self._key and event.event_type == "down":
            # Check modifiers
            active = set()
            if keyboard.is_pressed("ctrl"):
                active.add("ctrl")
            if keyboard.is_pressed("shift"):
                active.add("shift")
            if keyboard.is_pressed("alt"):
                active.add("alt")
            # Check left/right win keys by scan code
            if keyboard.is_pressed(91) or keyboard.is_pressed(92):
                active.add("win")
            if active == self._modifiers:
                self.triggered.emit()
                return False  # suppress the key
        return True  # pass through

    def stop(self):
        """Unregister the hotkey."""
        if self._hook is not None:
            keyboard.unhook(self._hook)
            self._hook = None

    def update_combo(self, combo: str):
        """Change the hotkey combo."""
        self._combo = combo
