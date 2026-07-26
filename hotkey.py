import sys

if sys.platform == "darwin":
    from hotkey_macos import GlobalHotkey, is_escape_pressed
elif sys.platform == "win32":
    from hotkey_windows import GlobalHotkey, is_escape_pressed
else:
    from hotkey_linux import GlobalHotkey, is_escape_pressed

__all__ = ["GlobalHotkey", "is_escape_pressed"]
