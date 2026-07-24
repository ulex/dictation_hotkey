import sys

if sys.platform == "darwin":
    from typing_macos import press_backspace, type_text
else:
    from typing_windows import press_backspace, type_text

__all__ = ["type_text", "press_backspace"]
