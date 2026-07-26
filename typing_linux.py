"""Linux text injection.

On Wayland, types through the zwp_virtual_keyboard_manager_v1 protocol
(supported by wlroots compositors such as Sway and by KDE KWin) using
pywayland. Arbitrary Unicode is typed by uploading a custom XKB keymap that
assigns spare keycodes to Unicode keysyms (the same trick wtype uses).

Fallback chain: native Wayland virtual keyboard -> `wtype` -> `ydotool`
(any session) -> `xdotool` (X11 only).
"""

import atexit
import os
import shutil
import subprocess
import sys
import time

# XKB keycodes usable for our custom mapping (evdev code = xkb code - 8).
_KEYCODE_MIN = 9
_KEYCODE_MAX = 255
_BACKSPACE_XKB_CODE = _KEYCODE_MAX  # reserved for the BackSpace keysym

_XKB_KEYMAP_FORMAT_V1 = 1
_KEY_RELEASED = 0
_KEY_PRESSED = 1

_SPECIAL_KEYSYMS = {
    "\n": "Return",
    "\r": "Return",
    "\t": "Tab",
    "\x1b": "Escape",
}


def _keysym_name(char: str) -> str | None:
    """Return the XKB keysym name for a character, or None if untypable."""
    if char in _SPECIAL_KEYSYMS:
        return _SPECIAL_KEYSYMS[char]
    cp = ord(char)
    if cp < 0x20 or cp == 0x7F:
        return None  # other control characters have no useful keysym
    return f"U{cp:04X}"


def _build_keymap(char_to_xkb_code: dict[str, int]) -> bytes:
    """Build an XKB keymap text assigning keycodes to Unicode keysyms."""
    codes = ["    <I%d> = %d;" % (c, c) for c in sorted(char_to_xkb_code.values())]
    keys = []
    for char, code in sorted(char_to_xkb_code.items(), key=lambda kv: kv[1]):
        keysym = _keysym_name(char)
        keys.append(
            '    key <I%d> { type= "ONE_LEVEL", symbols[Group1]= [ %s ] };'
            % (code, keysym)
        )
    keymap = (
        "xkb_keymap {\n"
        'xkb_keycodes "dictation" {\n'
        "    minimum = 8;\n"
        "    maximum = %d;\n" % _KEYCODE_MAX
        + "\n".join(codes)
        + "\n};\n"
        'xkb_types "dictation" {\n'
        "    virtual_modifiers NumLock,Alt,LevelThree,LAlt,RAlt,RControl,"
        "RShift,LVShift,LevelFive,Mouse,ScrollLock;\n"
        '    include "basic"\n'
        "};\n"
        'xkb_compat "dictation" {\n'
        '    include "complete"\n'
        "};\n"
        'xkb_symbols "dictation" {\n'
        '    name[Group1] = "Dictation";\n'
        + "\n".join(keys)
        + "\n};\n"
        "};\n"
    )
    return keymap.encode("utf-8")


class _WaylandVirtualKeyboard:
    """Types text via zwp_virtual_keyboard_manager_v1."""

    def __init__(self):
        from pywayland.client import Display

        from protocol.virtual_keyboard_unstable_v1 import ZwpVirtualKeyboardManagerV1
        from pywayland.protocol.wayland import WlSeat

        self._display = Display()
        self._display.connect()
        self._manager = None
        self._seat = None
        registry = self._display.get_registry()

        def on_global(reg, name, interface, version):
            if interface == ZwpVirtualKeyboardManagerV1.name and self._manager is None:
                self._manager = reg.bind(name, ZwpVirtualKeyboardManagerV1, 1)
            elif interface == WlSeat.name and self._seat is None:
                self._seat = reg.bind(name, WlSeat, min(version, 7))

        registry.dispatcher["global"] = on_global
        self._display.roundtrip()
        if self._manager is None:
            raise RuntimeError(
                "compositor does not support zwp_virtual_keyboard_manager_v1"
            )
        if self._seat is None:
            raise RuntimeError("compositor did not advertise a wl_seat")
        self._vkbd = self._manager.create_virtual_keyboard(self._seat)
        self._display.roundtrip()

    def _send_keymap(self, char_to_xkb_code: dict[str, int]):
        keymap = _build_keymap(char_to_xkb_code)
        fd = os.memfd_create("dictation-keymap")
        os.write(fd, keymap)
        os.lseek(fd, 0, os.SEEK_SET)
        self._vkbd.keymap(_XKB_KEYMAP_FORMAT_V1, fd, len(keymap))
        # Reset modifier state
        self._vkbd.modifiers(0, 0, 0, 0)
        self._display.flush()
        # NB: do NOT close(fd) — libwayland takes ownership of queued fds
        # and closes them once written to the socket.

    def _press(self, xkb_code: int, delay_ms: int = 2):
        evdev_code = xkb_code - 8
        now = int(time.monotonic() * 1000) & 0xFFFFFFFF
        self._vkbd.key(now, evdev_code, _KEY_PRESSED)
        self._vkbd.key(now + delay_ms, evdev_code, _KEY_RELEASED)

    def _type_batch(self, batch: list[str]):
        # One spare keycode per unique character in this batch.
        mapping: dict[str, int] = {}
        next_code = _KEYCODE_MIN
        for char in batch:
            if char not in mapping:
                mapping[char] = next_code
                next_code += 1
        self._send_keymap(mapping)
        for char in batch:
            self._press(mapping[char])
        self._display.flush()

    def type_text(self, text: str):
        chars = [c for c in text if _keysym_name(c) is not None]
        capacity = _BACKSPACE_XKB_CODE - _KEYCODE_MIN
        batch: list[str] = []
        batch_set: set[str] = set()
        for char in chars:
            if char not in batch_set and len(batch_set) >= capacity:
                self._type_batch(batch)
                batch, batch_set = [], set()
            batch.append(char)
            batch_set.add(char)
        if batch:
            self._type_batch(batch)

    def press_backspace(self, count: int = 1):
        self._send_keymap({"\b": _BACKSPACE_XKB_CODE})
        for _ in range(count):
            self._press(_BACKSPACE_XKB_CODE)
        self._display.flush()

    def close(self):
        try:
            self._vkbd.destroy()
            self._display.disconnect()
        except Exception:
            pass


# "\b" must map to the BackSpace keysym even though it is a control char.
_SPECIAL_KEYSYMS["\b"] = "BackSpace"

_vkbd: _WaylandVirtualKeyboard | None = None
_vkbd_failed = False


def _shutdown():
    """Close the Wayland connection cleanly (avoids gc-order crashes)."""
    global _vkbd
    if _vkbd is not None:
        _vkbd.close()
        _vkbd = None


atexit.register(_shutdown)


def _get_vkbd() -> _WaylandVirtualKeyboard | None:
    global _vkbd, _vkbd_failed
    if _vkbd is not None:
        return _vkbd
    if _vkbd_failed:
        return None
    try:
        _vkbd = _WaylandVirtualKeyboard()
        return _vkbd
    except Exception as e:
        _vkbd_failed = True
        print(f"Wayland virtual keyboard unavailable: {e}", file=sys.stderr)
        return None


def _is_wayland() -> bool:
    return bool(os.environ.get("WAYLAND_DISPLAY"))


def _run_tool(cmd: list[str]) -> bool:
    try:
        subprocess.run(cmd, check=True, capture_output=True, timeout=10)
        return True
    except (OSError, subprocess.SubprocessError) as e:
        print(f"Typing tool failed ({cmd[0]}): {e}", file=sys.stderr)
        return False


def type_text(text: str, char_delay: float = 0.005):
    """Type text into the focused window."""
    if not text:
        return
    if _is_wayland():
        vkbd = _get_vkbd()
        if vkbd is not None:
            vkbd.type_text(text)
            return
        if shutil.which("wtype") and _run_tool(["wtype", "-d", "1", "--", text]):
            return
        if shutil.which("ydotool") and _run_tool(["ydotool", "type", "--", text]):
            return
        print("No working Wayland typing backend (need compositor support for "
              "virtual-keyboard-v1, or wtype/ydotool installed)", file=sys.stderr)
    else:
        if shutil.which("xdotool"):
            _run_tool(["xdotool", "type", "--delay", "5", "--", text])
        elif shutil.which("ydotool"):
            _run_tool(["ydotool", "type", "--", text])
        else:
            print("No typing backend: install xdotool", file=sys.stderr)


def press_backspace(count: int = 1):
    """Send `count` Backspace key presses."""
    if count <= 0:
        return
    if _is_wayland():
        vkbd = _get_vkbd()
        if vkbd is not None:
            vkbd.press_backspace(count)
            return
        if shutil.which("wtype"):
            ok = True
            for _ in range(count):
                ok = _run_tool(["wtype", "-k", "BackSpace"]) and ok
            if ok:
                return
        if shutil.which("ydotool"):
            _run_tool(["ydotool", "key"] + ["14:1", "14:0"] * count)
            return
        print("No working Wayland backspace backend", file=sys.stderr)
    else:
        if shutil.which("xdotool"):
            _run_tool(["xdotool", "key"] + ["BackSpace"] * count)
        elif shutil.which("ydotool"):
            _run_tool(["ydotool", "key"] + ["14:1", "14:0"] * count)
        else:
            print("No backspace backend: install xdotool", file=sys.stderr)
