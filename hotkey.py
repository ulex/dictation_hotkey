import keyboard

from PySide6.QtCore import QObject, Signal


def _parse_combo(combo: str):
    """Parse 'Ctrl+Shift+F23' into (frozenset of modifier names, key name)."""
    parts = [p.strip().lower() for p in combo.split("+")]
    modifiers = frozenset(p for p in parts if p in ("ctrl", "shift", "alt", "win"))
    keys = [p for p in parts if p not in ("ctrl", "shift", "alt", "win")]
    if len(keys) != 1:
        raise ValueError(f"Expected exactly one non-modifier key, got: {keys}")
    return modifiers, keys[0]


def _get_active_modifiers() -> set[str]:
    """Return the set of currently held modifier names."""
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
    return active


class GlobalHotkey(QObject):
    """Registers system-wide hotkeys and emits `triggered` when any is pressed."""

    triggered = Signal()

    def __init__(self, combos: list[str] | None = None, parent=None):
        super().__init__(parent)
        self._combos = combos or []
        self._parsed: list[tuple[frozenset[str], str]] = []
        self._held: set[str] = set()
        self._hook = None

    def start(self):
        """Register the hotkeys via low-level keyboard hook."""
        self._parsed = [_parse_combo(c) for c in self._combos]
        self._held.clear()
        if self._parsed:
            self._hook = keyboard.hook(self._on_event, suppress=True)

    def _on_event(self, event: keyboard.KeyboardEvent):
        """Intercept every key event; suppress and fire on matching combo, pass others through."""
        if not event.name:
            return True
        name = event.name.lower()
        if event.event_type == "up":
            self._held.discard(name)
            return True
        if event.event_type != "down":
            return True
        is_repeat = name in self._held
        self._held.add(name)
        active = None
        for modifiers, key in self._parsed:
            if name == key:
                if active is None:
                    active = _get_active_modifiers()
                if active == modifiers:
                    if not is_repeat:
                        self.triggered.emit()
                    return False  # suppress the key (initial and repeats)
        return True  # pass through

    def stop(self):
        """Unregister the hotkey."""
        if self._hook is not None:
            keyboard.unhook(self._hook)
            self._hook = None

    def update_combos(self, combos: list[str]):
        """Change the hotkey combos."""
        self._combos = combos
