"""macOS global hotkey via a Quartz CGEventTap.

Requires Accessibility permission (System Settings -> Privacy & Security ->
Accessibility) for the process running the app, otherwise the tap cannot be
created and the hotkey will not fire.
"""

import sys
import threading
import time

import Quartz
from PySide6.QtCore import QObject, Signal

KEYCODE_ESCAPE = 53
KEYCODE_DELETE = 51

_MODIFIER_FLAGS = {
    "cmd": Quartz.kCGEventFlagMaskCommand,
    "command": Quartz.kCGEventFlagMaskCommand,
    "win": Quartz.kCGEventFlagMaskCommand,  # same physical key position as Win
    "meta": Quartz.kCGEventFlagMaskCommand,
    "ctrl": Quartz.kCGEventFlagMaskControl,
    "control": Quartz.kCGEventFlagMaskControl,
    "alt": Quartz.kCGEventFlagMaskAlternate,
    "opt": Quartz.kCGEventFlagMaskAlternate,
    "option": Quartz.kCGEventFlagMaskAlternate,
    "shift": Quartz.kCGEventFlagMaskShift,
}

_MODIFIER_MASK = (
    Quartz.kCGEventFlagMaskCommand
    | Quartz.kCGEventFlagMaskControl
    | Quartz.kCGEventFlagMaskAlternate
    | Quartz.kCGEventFlagMaskShift
)

# macOS ANSI keycodes
_KEYCODES = {
    "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6, "x": 7,
    "c": 8, "v": 9, "b": 11, "q": 12, "w": 13, "e": 14, "r": 15,
    "y": 16, "t": 17, "1": 18, "2": 19, "3": 20, "4": 21, "6": 22,
    "5": 23, "=": 24, "9": 25, "7": 26, "-": 27, "8": 28, "0": 29,
    "]": 30, "o": 31, "u": 32, "[": 33, "i": 34, "p": 35,
    "return": 36, "enter": 36, "l": 37, "j": 38, "'": 39, "k": 40,
    ";": 41, "\\": 42, ",": 43, "/": 44, "n": 45, "m": 46, ".": 47,
    "tab": 48, "space": 49, "`": 50, "delete": 51, "backspace": 51,
    "escape": 53, "esc": 53,
    "f17": 64, "f18": 79, "f19": 80, "f20": 90,
    "f5": 96, "f6": 97, "f7": 98, "f3": 99, "f8": 100, "f9": 101,
    "f11": 103, "f13": 105, "f16": 106, "f14": 107, "f10": 109,
    "f12": 111, "f15": 113, "f4": 118, "f2": 120, "f1": 122,
}


def is_escape_pressed() -> bool:
    """Return True while the Escape key is held down."""
    return Quartz.CGEventSourceKeyState(
        Quartz.kCGEventSourceStateHIDSystemState, KEYCODE_ESCAPE
    )


def _parse_combo(combo: str):
    """Parse 'Cmd+Shift+D' into (modifier flag mask, macOS keycode)."""
    parts = [p.strip().lower() for p in combo.split("+") if p.strip()]
    flags = 0
    keys = []
    for p in parts:
        if p in _MODIFIER_FLAGS:
            flags |= _MODIFIER_FLAGS[p]
        else:
            keys.append(p)
    if len(keys) != 1:
        raise ValueError(f"Expected exactly one non-modifier key, got: {keys}")
    if keys[0] not in _KEYCODES:
        raise ValueError(f"Unsupported key name: {keys[0]!r} (in {combo!r})")
    return flags, _KEYCODES[keys[0]]


class GlobalHotkey(QObject):
    """Registers system-wide hotkeys and emits `triggered` when any is pressed.

    Uses a Quartz event tap on a background thread's CFRunLoop. Matching key
    presses are suppressed; everything else passes through.
    """

    triggered = Signal()

    def __init__(self, combos: list[str] | None = None, parent=None):
        super().__init__(parent)
        self._combos = combos or []
        self._parsed: list[tuple[int, int]] = []
        self._last_trigger = 0.0
        self._tap = None
        self._run_loop = None
        self._thread: threading.Thread | None = None
        self._lock = threading.Lock()

    def start(self):
        """Install the event tap."""
        self._parsed = [_parse_combo(c) for c in self._combos]
        self._last_trigger = 0.0
        if not self._parsed:
            return
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    def _run(self):
        mask = Quartz.CGEventMaskBit(Quartz.kCGEventKeyDown)
        tap = Quartz.CGEventTapCreate(
            Quartz.kCGSessionEventTap,
            Quartz.kCGHeadInsertEventTap,
            Quartz.kCGEventTapOptionDefault,
            mask,
            self._callback,
            None,
        )
        if tap is None:
            print(
                "GlobalHotkey: failed to create event tap. Grant Accessibility "
                "permission (System Settings -> Privacy & Security -> "
                "Accessibility) and restart the app.",
                file=sys.stderr,
            )
            return
        with self._lock:
            self._tap = tap
            self._run_loop = Quartz.CFRunLoopGetCurrent()
        source = Quartz.CFMachPortCreateRunLoopSource(None, tap, 0)
        Quartz.CFRunLoopAddSource(self._run_loop, source, Quartz.kCFRunLoopCommonModes)
        Quartz.CGEventTapEnable(tap, True)
        Quartz.CFRunLoopRun()
        # Run loop stopped: clean up
        Quartz.CFMachPortInvalidate(tap)
        with self._lock:
            self._tap = None
            self._run_loop = None

    def _callback(self, proxy, event_type, event, refcon):
        if event_type == Quartz.kCGEventTapDisabledByTimeout:
            with self._lock:
                tap = self._tap
            if tap is not None:
                Quartz.CGEventTapEnable(tap, True)
            return event
        if event_type != Quartz.kCGEventKeyDown:
            return event
        keycode = Quartz.CGEventGetIntegerValueField(
            event, Quartz.kCGKeyboardEventKeycode
        )
        flags = Quartz.CGEventGetFlags(event) & _MODIFIER_MASK
        now = time.monotonic()
        for want_flags, want_keycode in self._parsed:
            if keycode == want_keycode and flags == want_flags:
                if now - self._last_trigger > 1.0:
                    self._last_trigger = now
                    self.triggered.emit()
                return None  # suppress the key press (initial and repeats)
        return event

    def stop(self):
        """Remove the event tap."""
        with self._lock:
            run_loop = self._run_loop
        if run_loop is not None:
            Quartz.CFRunLoopStop(run_loop)
        if self._thread is not None:
            self._thread.join(timeout=2.0)
            self._thread = None

    def update_combos(self, combos: list[str]):
        """Change the hotkey combos."""
        self._combos = combos
