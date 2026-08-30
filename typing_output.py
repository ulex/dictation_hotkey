import ctypes
import ctypes.wintypes
import time

from PySide6.QtWidgets import QApplication

# Constants
INPUT_KEYBOARD = 1
KEYEVENTF_UNICODE = 0x0004
KEYEVENTF_KEYUP = 0x0002
VK_CONTROL = 0x11
VK_SHIFT = 0x10
VK_V = 0x56
VK_INSERT = 0x2D

# Paste shortcut presets: name -> list of VK codes (modifiers first)
PASTE_SHORTCUTS = {
    "shift_insert": [VK_SHIFT, VK_INSERT],
    "ctrl_v": [VK_CONTROL, VK_V],
    "ctrl_shift_v": [VK_CONTROL, VK_SHIFT, VK_V],
}
DEFAULT_PASTE_SHORTCUT = "shift_insert"

MODE_PASTE = "paste"
MODE_KEYSTROKES = "keystrokes"


class MOUSEINPUT(ctypes.Structure):
    _fields_ = [
        ("dx", ctypes.wintypes.LONG),
        ("dy", ctypes.wintypes.LONG),
        ("mouseData", ctypes.wintypes.DWORD),
        ("dwFlags", ctypes.wintypes.DWORD),
        ("time", ctypes.wintypes.DWORD),
        ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong)),
    ]


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [
        ("wVk", ctypes.wintypes.WORD),
        ("wScan", ctypes.wintypes.WORD),
        ("dwFlags", ctypes.wintypes.DWORD),
        ("time", ctypes.wintypes.DWORD),
        ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong)),
    ]


class HARDWAREINPUT(ctypes.Structure):
    _fields_ = [
        ("uMsg", ctypes.wintypes.DWORD),
        ("wParamL", ctypes.wintypes.WORD),
        ("wParamH", ctypes.wintypes.WORD),
    ]


class INPUT(ctypes.Structure):
    class _INPUT(ctypes.Union):
        _fields_ = [
            ("mi", MOUSEINPUT),
            ("ki", KEYBDINPUT),
            ("hi", HARDWAREINPUT),
        ]

    _anonymous_ = ("_input",)
    _fields_ = [
        ("type", ctypes.wintypes.DWORD),
        ("_input", _INPUT),
    ]


_SendInput = ctypes.windll.user32.SendInput
_SendInput.argtypes = [ctypes.c_uint, ctypes.POINTER(INPUT), ctypes.c_int]
_SendInput.restype = ctypes.c_uint


def _key_event(inputs, i, vk, scan, flags):
    inputs[i].type = INPUT_KEYBOARD
    inputs[i].ki.wVk = vk
    inputs[i].ki.wScan = scan
    inputs[i].ki.dwFlags = flags


def _send_combo(vks):
    """Press a key combination as one atomic SendInput batch.

    `vks` lists keys in press order (modifiers first); keys are released
    in reverse order.
    """
    n = len(vks)
    inputs = (INPUT * (2 * n))()
    for i, vk in enumerate(vks):
        _key_event(inputs, i, vk, 0, 0)
        _key_event(inputs, 2 * n - 1 - i, vk, 0, KEYEVENTF_KEYUP)
    _SendInput(2 * n, inputs, ctypes.sizeof(INPUT))


def _send_ctrl_v():
    """Send a single Ctrl+V keystroke (one atomic SendInput batch)."""
    _send_combo([VK_CONTROL, VK_V])


def _type_chars(text: str, char_delay: float):
    """Type text into the focused window using SendInput Unicode events.

    All events are sent in ONE SendInput call: ordering within a single
    call is guaranteed and other input cannot interleave, which avoids the
    character reordering seen with many small calls (e.g. over RDP).
    """
    MAX_EVENTS = 500  # send in chunks to stay well under SendInput limits
    pending = []
    for char in text:
        pending.append((ord(char), KEYEVENTF_UNICODE))
        pending.append((ord(char), KEYEVENTF_UNICODE | KEYEVENTF_KEYUP))
    for start in range(0, len(pending), MAX_EVENTS):
        chunk = pending[start:start + MAX_EVENTS]
        inputs = (INPUT * len(chunk))()
        for i, (code, flags) in enumerate(chunk):
            _key_event(inputs, i, 0, code, flags)
        _SendInput(len(chunk), inputs, ctypes.sizeof(INPUT))
        if char_delay > 0 and start + MAX_EVENTS < len(pending):
            time.sleep(char_delay)


def _paste_text(text: str, shortcut: str = DEFAULT_PASTE_SHORTCUT):
    """Set the clipboard to `text` and send the paste shortcut.

    The dictated text stays on the clipboard afterwards (no restore),
    which also avoids races with RDP delayed clipboard rendering.
    """
    QApplication.clipboard().setText(text)
    _send_combo(PASTE_SHORTCUTS.get(shortcut, PASTE_SHORTCUTS[DEFAULT_PASTE_SHORTCUT]))


def type_text(text: str, mode: str = MODE_PASTE, paste_shortcut: str = DEFAULT_PASTE_SHORTCUT,
              char_delay: float = 0.005):
    """Type text into the focused window.

    mode="paste": clipboard + paste shortcut (atomic, reliable over Remote
        Desktop). `paste_shortcut` selects the key combo (see PASTE_SHORTCUTS).
    mode="keystrokes": batched SendInput Unicode events.
    """
    if not text:
        return
    if mode == MODE_PASTE:
        _paste_text(text, paste_shortcut)
    else:
        _type_chars(text, char_delay)
