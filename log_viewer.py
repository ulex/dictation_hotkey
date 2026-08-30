from PySide6.QtCore import Qt
from PySide6.QtGui import QFont
from PySide6.QtWidgets import (
    QDialog, QVBoxLayout, QHBoxLayout,
    QPlainTextEdit, QPushButton,
)

import log_buffer


class LogViewerDialog(QDialog):
    def __init__(self, parent=None):
        super().__init__(parent)
        self.setWindowTitle("Dictation Hotkey — Logs")
        self.setMinimumSize(720, 420)
        self.setWindowFlag(Qt.WindowType.WindowStaysOnTopHint, False)

        layout = QVBoxLayout(self)

        self._text = QPlainTextEdit()
        self._text.setReadOnly(True)
        self._text.setFont(QFont("Consolas", 9))
        self._text.setMaximumBlockCount(2000)
        layout.addWidget(self._text)

        btn_row = QHBoxLayout()
        clear_btn = QPushButton("Clear")
        clear_btn.clicked.connect(self._clear)
        btn_row.addWidget(clear_btn)
        btn_row.addStretch()
        close_btn = QPushButton("Close")
        close_btn.clicked.connect(self.hide)
        btn_row.addWidget(close_btn)
        layout.addLayout(btn_row)

        for line in log_buffer.get_all():
            self._text.appendPlainText(line)
        self._scroll_to_bottom()

        log_buffer._emitter.new_line.connect(self._on_new_line)

    def _on_new_line(self, line: str) -> None:
        self._text.appendPlainText(line)
        self._scroll_to_bottom()

    def _scroll_to_bottom(self) -> None:
        sb = self._text.verticalScrollBar()
        sb.setValue(sb.maximum())

    def _clear(self) -> None:
        log_buffer.clear()
        self._text.clear()

    def show_and_raise(self) -> None:
        self.show()
        self.raise_()
        self.activateWindow()
