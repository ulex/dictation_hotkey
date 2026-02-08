from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QDialog, QFormLayout, QLineEdit, QPushButton,
    QDialogButtonBox, QLabel, QCheckBox, QGroupBox, QVBoxLayout,
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

        # Hotkey group
        hotkey_group = QGroupBox("Hotkeys")
        hotkey_layout = QVBoxLayout(hotkey_group)

        self._copilot_cb = QCheckBox("Use Copilot key (Win+C, Win+Shift+F23)")
        self._copilot_cb.setChecked(self._config.get("hotkey_copilot", False))
        hotkey_layout.addWidget(self._copilot_cb)

        self._win_h_cb = QCheckBox("Replace Windows dictation (Win+H)")
        self._win_h_cb.setChecked(self._config.get("hotkey_win_h", True))
        hotkey_layout.addWidget(self._win_h_cb)

        self._custom_edit = QLineEdit(self._config.get("hotkey_custom", ""))
        self._custom_edit.setPlaceholderText("e.g. Win+Y")
        custom_form = QFormLayout()
        custom_form.addRow("Custom shortcut:", self._custom_edit)
        hotkey_layout.addLayout(custom_form)

        layout.addRow(hotkey_group)

        # Language (optional)
        self._language_edit = QLineEdit(self._config.get("language", ""))
        self._language_edit.setPlaceholderText("e.g. en (leave blank for auto)")
        layout.addRow("Language:", self._language_edit)

        # Start with Windows
        self._startup_cb = QCheckBox("Start with Windows")
        self._startup_cb.setChecked(self._config.get("start_with_windows", False))
        layout.addRow(self._startup_cb)

        # Buttons
        buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel
        )
        buttons.accepted.connect(self._on_ok)
        buttons.rejected.connect(self.reject)
        layout.addRow(buttons)

    def _on_ok(self):
        self._config["api_key"] = self._api_key_edit.text().strip()
        self._config["hotkey_copilot"] = self._copilot_cb.isChecked()
        self._config["hotkey_win_h"] = self._win_h_cb.isChecked()
        self._config["hotkey_custom"] = self._custom_edit.text().strip()
        self._config["language"] = self._language_edit.text().strip()
        new_startup = self._startup_cb.isChecked()
        if new_startup != self._config.get("start_with_windows", False):
            config.set_startup_shortcut(new_startup)
        self._config["start_with_windows"] = new_startup
        config.save(self._config)
        self.accept()

    def get_config(self) -> dict:
        return self._config
