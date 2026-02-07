from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QDialog, QFormLayout, QLineEdit, QPushButton,
    QDialogButtonBox, QLabel,
)

import config


class SettingsDialog(QDialog):
    def __init__(self, current_config: dict, parent=None):
        super().__init__(parent)
        self.setWindowTitle("Dictation Hotkey Settings")
        self.setMinimumWidth(380)
        self._config = dict(current_config)

        layout = QFormLayout(self)

        # API key
        self._api_key_edit = QLineEdit(self._config.get("api_key", ""))
        self._api_key_edit.setEchoMode(QLineEdit.EchoMode.Password)
        self._api_key_edit.setPlaceholderText("Enter Mistral API key")
        layout.addRow("API Key:", self._api_key_edit)

        # Hotkey
        self._hotkey_edit = QLineEdit(self._config.get("hotkey", "Win+Y"))
        self._hotkey_edit.setPlaceholderText("e.g. Win+Y, Ctrl+Shift+D")
        layout.addRow("Hotkey:", self._hotkey_edit)

        # Language (optional)
        self._language_edit = QLineEdit(self._config.get("language", ""))
        self._language_edit.setPlaceholderText("e.g. en (leave blank for auto)")
        layout.addRow("Language:", self._language_edit)

        # Buttons
        buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel
        )
        buttons.accepted.connect(self._on_ok)
        buttons.rejected.connect(self.reject)
        layout.addRow(buttons)

    def _on_ok(self):
        self._config["api_key"] = self._api_key_edit.text().strip()
        self._config["hotkey"] = self._hotkey_edit.text().strip() or "Win+Y"
        self._config["language"] = self._language_edit.text().strip()
        config.save(self._config)
        self.accept()

    def get_config(self) -> dict:
        return self._config
