#!/usr/bin/env python3
"""Unit tests for the Linux evdev hotkey backend (no real devices needed).

Feeds synthetic input events into GlobalHotkey._on_key_event and checks
triggering and suppression behaviour.

Run:  python tests/test_hotkey_linux.py
"""

import os
import sys
import types
import time

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from evdev import ecodes

import hotkey_linux
from hotkey_linux import GlobalHotkey, _parse_combo, is_escape_pressed


def ev(code, value):
    return types.SimpleNamespace(type=ecodes.EV_KEY, code=code, value=value)


def make_hotkey(combos):
    hk = GlobalHotkey(combos=combos)
    hk._parsed = [_parse_combo(c) for c in combos]
    fired = []
    hk.triggered.connect(lambda: fired.append(time.monotonic()))
    return hk, fired


def check(cond, msg):
    if not cond:
        raise AssertionError(msg)
    print(f"  ok: {msg}")


def test_parse_combo():
    print("test_parse_combo")
    mods, key = _parse_combo("Alt+Space")
    check(mods == frozenset({"alt"}) and key == ecodes.KEY_SPACE, "Alt+Space")
    mods, key = _parse_combo("Ctrl+Shift+D")
    check(mods == frozenset({"ctrl", "shift"}) and key == ecodes.KEY_D, "Ctrl+Shift+D")
    mods, key = _parse_combo("Super+Y")
    check(mods == frozenset({"win"}) and key == ecodes.KEY_Y, "Super+Y -> win")
    mods, key = _parse_combo("Control+F13")
    check(mods == frozenset({"ctrl"}) and key == ecodes.KEY_F13, "alias canonicalisation")
    try:
        _parse_combo("Alt+")
        raise SystemExit("expected ValueError for modifier-only combo")
    except ValueError:
        check(True, "modifier-only combo rejected")
    try:
        _parse_combo("Alt+NoSuchKey")
        raise SystemExit("expected ValueError for unknown key")
    except ValueError:
        check(True, "unknown key rejected")


def test_trigger_and_suppress():
    print("test_trigger_and_suppress")
    hk, fired = make_hotkey(["Alt+Space"])

    check(hk._on_key_event(ev(ecodes.KEY_A, 1)) is False, "plain key not suppressed")
    check(hk._on_key_event(ev(ecodes.KEY_A, 0)) is False, "plain key release not suppressed")
    check(not fired, "plain key does not fire")

    check(hk._on_key_event(ev(ecodes.KEY_LEFTALT, 1)) is False, "alt down forwarded")
    check(hk._on_key_event(ev(ecodes.KEY_SPACE, 1)) is True, "hotkey down suppressed")
    check(len(fired) == 1, "hotkey fired once")
    check(hk._on_key_event(ev(ecodes.KEY_SPACE, 2)) is True, "hotkey repeat suppressed")
    check(len(fired) == 1, "repeat does not fire again")
    check(hk._on_key_event(ev(ecodes.KEY_SPACE, 0)) is True, "hotkey release suppressed")
    check(hk._on_key_event(ev(ecodes.KEY_LEFTALT, 0)) is False, "alt release forwarded")

    # After the full press, state is clean again
    check(not hk._pressed, "no stuck pressed keys")
    check(not hk._suppressed, "no stuck suppressed keys")


def test_wrong_modifiers():
    print("test_wrong_modifiers")
    hk, fired = make_hotkey(["Ctrl+Shift+D"])
    hk._on_key_event(ev(ecodes.KEY_LEFTCTRL, 1))
    check(hk._on_key_event(ev(ecodes.KEY_D, 1)) is False, "missing shift -> not suppressed")
    check(not fired, "missing shift -> no fire")
    hk._on_key_event(ev(ecodes.KEY_LEFTSHIFT, 1))
    hk._last_trigger = 0  # reset debounce
    check(hk._on_key_event(ev(ecodes.KEY_D, 1)) is True, "full combo suppressed")
    check(len(fired) == 1, "full combo fired")


def test_debounce():
    print("test_debounce")
    hk, fired = make_hotkey(["Alt+Space"])
    hk._on_key_event(ev(ecodes.KEY_LEFTALT, 1))
    hk._on_key_event(ev(ecodes.KEY_SPACE, 1))
    hk._on_key_event(ev(ecodes.KEY_SPACE, 0))
    hk._on_key_event(ev(ecodes.KEY_SPACE, 1))  # quick re-press within 1s
    check(len(fired) == 1, "second press within debounce window ignored")


def test_escape_tracking():
    print("test_escape_tracking")
    hk, _ = make_hotkey(["Alt+Space"])
    hk._on_key_event(ev(ecodes.KEY_ESC, 1))
    check(is_escape_pressed(), "escape down tracked")
    hk._on_key_event(ev(ecodes.KEY_ESC, 0))
    check(not is_escape_pressed(), "escape up tracked")


def main():
    test_parse_combo()
    test_trigger_and_suppress()
    test_wrong_modifiers()
    test_debounce()
    test_escape_tracking()
    print("PASS: all hotkey_linux unit tests")


if __name__ == "__main__":
    main()
