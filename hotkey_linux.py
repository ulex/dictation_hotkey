"""Linux global hotkey via evdev — display-server agnostic (Wayland and X11).

Keyboard events are read directly from /dev/input/event* devices, so no
support from the Wayland compositor or X server is needed.

Two modes:

- **Grab mode** (default): devices are grabbed exclusively and their events
  re-emitted through a uinput virtual keyboard. Matching hotkey combos are
  suppressed, i.e. they never reach the focused application — same behaviour
  as the Windows/macOS backends. Requires write access to /dev/uinput.
- **Passive mode** (fallback when uinput is unavailable): devices are only
  listened to. Combos still fire, but the keypress also reaches the focused
  application.

Permissions: the user needs read access to /dev/input/event* (usually via
the `input` group) and, for grab mode, read/write access to /dev/uinput.
See README.md for the required udev rule.
"""

import selectors
import sys
import threading
import time

from PySide6.QtCore import QObject, Signal

_MODIFIERS = {
    "ctrl": ("KEY_LEFTCTRL", "KEY_RIGHTCTRL"),
    "control": ("KEY_LEFTCTRL", "KEY_RIGHTCTRL"),
    "shift": ("KEY_LEFTSHIFT", "KEY_RIGHTSHIFT"),
    "alt": ("KEY_LEFTALT", "KEY_RIGHTALT"),
    "opt": ("KEY_LEFTALT", "KEY_RIGHTALT"),
    "option": ("KEY_LEFTALT", "KEY_RIGHTALT"),
    "win": ("KEY_LEFTMETA", "KEY_RIGHTMETA"),
    "super": ("KEY_LEFTMETA", "KEY_RIGHTMETA"),
    "meta": ("KEY_LEFTMETA", "KEY_RIGHTMETA"),
    "cmd": ("KEY_LEFTMETA", "KEY_RIGHTMETA"),
}

_KEY_ALIASES = {
    "space": "KEY_SPACE",
    "enter": "KEY_ENTER",
    "return": "KEY_ENTER",
    "esc": "KEY_ESC",
    "escape": "KEY_ESC",
    "tab": "KEY_TAB",
    "backspace": "KEY_BACKSPACE",
    "delete": "KEY_DELETE",
    "del": "KEY_DELETE",
}

_VKBD_NAME = "dictation-hotkey-virtual-keyboard"

# Updated by the active GlobalHotkey instance; read by is_escape_pressed().
_escape_pressed = False


def is_escape_pressed() -> bool:
    """Return True while the Escape key is held down."""
    return _escape_pressed


def _ecodes():
    from evdev import ecodes

    return ecodes


def _resolve_key(name: str) -> int:
    """Resolve a key name like 'd', 'space', 'f13' to an evdev key code."""
    ecodes = _ecodes()
    if name in _KEY_ALIASES:
        return ecodes.ecodes[_KEY_ALIASES[name]]
    ecode_name = name if name.startswith("KEY_") else "KEY_" + name.upper()
    if ecode_name not in ecodes.ecodes:
        raise ValueError(f"Unsupported key name: {name!r}")
    return ecodes.ecodes[ecode_name]


def _parse_combo(combo: str):
    """Parse 'Ctrl+Shift+D' into (frozenset of modifier names, evdev key code)."""
    parts = [p.strip().lower() for p in combo.split("+") if p.strip()]
    modifiers = set()
    keys = []
    for p in parts:
        if p in _MODIFIERS:
            # Canonical name (first alias) so 'control' and 'ctrl' compare equal
            canonical = {"control": "ctrl", "opt": "alt", "option": "alt",
                         "super": "win", "meta": "win", "cmd": "win"}.get(p, p)
            modifiers.add(canonical)
        else:
            keys.append(p)
    if len(keys) != 1:
        raise ValueError(f"Expected exactly one non-modifier key, got: {keys}")
    return frozenset(modifiers), _resolve_key(keys[0])


def _modifier_codes() -> dict[int, str]:
    """Map evdev key code -> canonical modifier name."""
    ecodes = _ecodes()
    result = {}
    for canonical, names in (
        ("ctrl", _MODIFIERS["ctrl"]),
        ("shift", _MODIFIERS["shift"]),
        ("alt", _MODIFIERS["alt"]),
        ("win", _MODIFIERS["win"]),
    ):
        for n in names:
            result[ecodes.ecodes[n]] = canonical
    return result


class GlobalHotkey(QObject):
    """Registers system-wide hotkeys and emits `triggered` when any is pressed."""

    triggered = Signal()

    def __init__(self, combos: list[str] | None = None, parent=None):
        super().__init__(parent)
        self._combos = combos or []
        self._parsed: list[tuple[frozenset[str], int]] = []
        self._last_trigger = 0.0
        self._thread: threading.Thread | None = None
        self._running = False
        self._devices = []
        self._uinput = None
        self._grabbed = False
        self._pressed: set[int] = set()
        self._suppressed: set[int] = set()
        self._mod_codes = _modifier_codes()

    # -- setup ------------------------------------------------------------

    def _find_keyboards(self):
        """Open all real keyboard devices (excludes our own virtual one)."""
        from evdev import InputDevice, ecodes, list_devices

        devices = []
        for path in list_devices():
            try:
                dev = InputDevice(path)
            except OSError:
                continue
            try:
                if dev.name == _VKBD_NAME:
                    dev.close()
                    continue
                keys = dev.capabilities().get(ecodes.EV_KEY, ())
                if ecodes.KEY_A in keys and ecodes.KEY_Z in keys:
                    devices.append(dev)
                else:
                    dev.close()
            except OSError:
                dev.close()
        return devices

    def start(self):
        """Start listening for the hotkeys."""
        global _escape_pressed
        from evdev import UInput, ecodes

        _escape_pressed = False
        self._parsed = [_parse_combo(c) for c in self._combos]
        self._last_trigger = 0.0
        if not self._parsed:
            return

        self._devices = self._find_keyboards()
        if not self._devices:
            print(
                "GlobalHotkey: no readable keyboard devices found. Grant read "
                "access to /dev/input/event* (e.g. add your user to the "
                "'input' group) and restart the app.",
                file=sys.stderr,
            )
            return

        # Try to set up grab + replay so combos can be suppressed.
        self._uinput = None
        all_keys = set()
        for dev in self._devices:
            all_keys.update(dev.capabilities().get(ecodes.EV_KEY, ()))
        try:
            self._uinput = UInput(
                events={ecodes.EV_KEY: sorted(all_keys)},
                name=_VKBD_NAME,
            )
        except (OSError, PermissionError) as e:
            print(
                f"GlobalHotkey: cannot create uinput device ({e}); falling "
                "back to passive mode — hotkey keypresses will also reach "
                "the focused application. Grant access to /dev/uinput to fix.",
                file=sys.stderr,
            )

        if self._uinput is not None:
            for dev in self._devices:
                try:
                    dev.grab()
                except OSError as e:
                    print(f"GlobalHotkey: failed to grab {dev.path}: {e}",
                          file=sys.stderr)
            self._grabbed = True

        self._running = True
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    # -- event loop --------------------------------------------------------

    def _run(self):
        from evdev import ecodes

        selector = selectors.DefaultSelector()
        for dev in self._devices:
            selector.register(dev, selectors.EVENT_READ)
        try:
            while self._running:
                for key, _ in selector.select(timeout=0.2):
                    dev = key.fileobj
                    try:
                        events = dev.read()
                    except OSError:
                        # Device disconnected
                        selector.unregister(dev)
                        continue
                    for event in events:
                        if event.type == ecodes.EV_KEY:
                            self._on_key_event(event)
        finally:
            selector.close()

    def _on_key_event(self, event):
        global _escape_pressed
        from evdev import ecodes

        code, value = event.code, event.value

        if code == ecodes.KEY_ESC:
            _escape_pressed = value != 0

        # Track modifier state
        if value == 1:
            self._pressed.add(code)
        elif value == 0:
            self._pressed.discard(code)

        suppress = False
        if value in (1, 2):
            active_mods = {
                self._mod_codes[c] for c in self._pressed if c in self._mod_codes
            }
            now = time.monotonic()
            for modifiers, keycode in self._parsed:
                if code == keycode and active_mods == modifiers:
                    suppress = True
                    if value == 1 and now - self._last_trigger > 1.0:
                        self._last_trigger = now
                        self.triggered.emit()
                    break
        if value == 1 and suppress:
            self._suppressed.add(code)
        elif value == 0 and code in self._suppressed:
            self._suppressed.discard(code)
            suppress = True

        if self._grabbed and self._uinput is not None and not suppress:
            try:
                self._uinput.write(ecodes.EV_KEY, code, value)
                self._uinput.syn()
            except OSError as e:
                print(f"GlobalHotkey: uinput write failed: {e}", file=sys.stderr)

        return suppress  # exposed for tests

    # -- teardown ----------------------------------------------------------

    def stop(self):
        """Stop listening and release all devices."""
        self._running = False
        if self._thread is not None:
            self._thread.join(timeout=2.0)
            self._thread = None
        for dev in self._devices:
            try:
                if self._grabbed:
                    dev.ungrab()
            except OSError:
                pass
            try:
                dev.close()
            except OSError:
                pass
        self._devices = []
        self._grabbed = False
        if self._uinput is not None:
            self._uinput.close()
            self._uinput = None

    def update_combos(self, combos: list[str]):
        """Change the hotkey combos."""
        self._combos = combos
