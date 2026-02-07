import ctypes
import ctypes.wintypes
import threading

from PySide6.QtCore import QObject, Signal

# Win32 constants
MOD_ALT = 0x0001
MOD_CONTROL = 0x0002
MOD_SHIFT = 0x0004
MOD_WIN = 0x0008
WM_HOTKEY = 0x0312

# Map modifier names to Win32 flags
_MOD_MAP = {
    "ctrl": MOD_CONTROL,
    "alt": MOD_ALT,
    "shift": MOD_SHIFT,
    "win": MOD_WIN,
}

# Map key names to virtual-key codes (common subset)
_VK_MAP = {chr(c): c for c in range(0x41, 0x5B)}  # A-Z
_VK_MAP.update({str(i): 0x30 + i for i in range(10)})  # 0-9
_VK_MAP.update({
    "F1": 0x70, "F2": 0x71, "F3": 0x72, "F4": 0x73,
    "F5": 0x74, "F6": 0x75, "F7": 0x76, "F8": 0x77,
    "F9": 0x78, "F10": 0x79, "F11": 0x7A, "F12": 0x7B,
    "SPACE": 0x20, "ENTER": 0x0D, "TAB": 0x09,
})


def parse_hotkey(combo: str):
    """Parse a hotkey string like 'Ctrl+Shift+D' into (modifiers, vk)."""
    parts = [p.strip().upper() for p in combo.split("+")]
    modifiers = 0
    vk = 0
    for part in parts:
        low = part.lower()
        if low in _MOD_MAP:
            modifiers |= _MOD_MAP[low]
        elif part in _VK_MAP:
            vk = _VK_MAP[part]
        else:
            raise ValueError(f"Unknown key: {part}")
    return modifiers, vk


class GlobalHotkey(QObject):
    """Registers a system-wide hotkey and emits `triggered` when pressed."""

    triggered = Signal()

    def __init__(self, combo: str = "Win+Y", parent=None):
        super().__init__(parent)
        self._combo = combo
        self._hotkey_id = 1
        self._thread: threading.Thread | None = None
        self._registered = False

    def start(self):
        """Register the hotkey and start the message-pump thread."""
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    def _run(self):
        modifiers, vk = parse_hotkey(self._combo)
        user32 = ctypes.windll.user32
        if not user32.RegisterHotKey(None, self._hotkey_id, modifiers, vk):
            return
        self._registered = True
        msg = ctypes.wintypes.MSG()
        while user32.GetMessageW(ctypes.byref(msg), None, 0, 0) != 0:
            if msg.message == WM_HOTKEY:
                self.triggered.emit()
            user32.TranslateMessage(ctypes.byref(msg))
            user32.DispatchMessageW(ctypes.byref(msg))

    def stop(self):
        """Unregister the hotkey."""
        if self._registered:
            ctypes.windll.user32.UnregisterHotKey(None, self._hotkey_id)
            self._registered = False

    def update_combo(self, combo: str):
        """Change the hotkey combo (requires restart)."""
        self._combo = combo
