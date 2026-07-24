"""macOS text injection via Quartz CGEvents.

Posting events at the HID tap requires Accessibility permission, the same
permission already needed for the global hotkey event tap.
"""

import time

import Quartz

KEYCODE_DELETE = 51


def _post_unicode(char: str, key_down: bool):
    event = Quartz.CGEventCreateKeyboardEvent(None, 0, key_down)
    # Length is in UTF-16 code units (UniChars), not Python characters
    utf16_units = len(char.encode("utf-16-le")) // 2
    Quartz.CGEventKeyboardSetUnicodeString(event, utf16_units, char)
    Quartz.CGEventPost(Quartz.kCGHIDEventTap, event)


def type_text(text: str, char_delay: float = 0.005):
    """Type text into the focused window using Unicode keyboard events."""
    for char in text:
        _post_unicode(char, True)
        _post_unicode(char, False)
        if char_delay > 0:
            time.sleep(char_delay)


def press_backspace(count: int = 1):
    """Send `count` Delete (backspace) key presses."""
    for _ in range(count):
        down = Quartz.CGEventCreateKeyboardEvent(None, KEYCODE_DELETE, True)
        Quartz.CGEventPost(Quartz.kCGHIDEventTap, down)
        up = Quartz.CGEventCreateKeyboardEvent(None, KEYCODE_DELETE, False)
        Quartz.CGEventPost(Quartz.kCGHIDEventTap, up)
        time.sleep(0.005)
