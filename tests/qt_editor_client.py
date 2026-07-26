#!/usr/bin/env python3
"""A minimal Qt text editor used as the keyboard-focus target in the
Wayland end-to-end test.

Opens a QTextEdit, then periodically writes its content to the file named
in the TEST_OUT environment variable. Exits after TEST_TIMEOUT seconds
(default 60) or when closed.
"""

import os
import sys

from PySide6.QtCore import QEvent, QObject, QTimer
from PySide6.QtWidgets import QApplication, QTextEdit


class KeyLogger(QObject):
    def __init__(self, path):
        super().__init__()
        self._path = path

    def eventFilter(self, obj, event):
        if event.type() in (QEvent.Type.KeyPress, QEvent.Type.KeyRelease):
            try:
                with open(self._path, "a", encoding="utf-8") as f:
                    f.write(
                        f"{event.type().name} key=0x{event.key():x} "
                        f"text={event.text()!r}\n")
            except OSError:
                pass
        return False


def main():
    out_path = os.environ.get("TEST_OUT", "/tmp/dictation_qt_editor_out.txt")
    timeout_s = float(os.environ.get("TEST_TIMEOUT", "60"))

    app = QApplication(sys.argv)
    keys_path = os.environ.get("TEST_KEYS")
    if keys_path:
        logger = KeyLogger(keys_path)
        app.installEventFilter(logger)
    editor = QTextEdit()
    editor.setPlainText("")
    editor.resize(400, 300)
    editor.show()

    def dump():
        try:
            with open(out_path, "w", encoding="utf-8") as f:
                f.write(editor.toPlainText())
        except OSError:
            pass

    timer = QTimer()
    timer.setInterval(100)
    timer.timeout.connect(dump)
    timer.start()

    quit_timer = QTimer()
    quit_timer.setSingleShot(True)
    quit_timer.timeout.connect(app.quit)
    quit_timer.start(int(timeout_s * 1000))

    print("EDITOR_READY", flush=True)
    sys.exit(app.exec())


if __name__ == "__main__":
    main()
