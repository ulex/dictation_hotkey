import json
import os
import subprocess
import sys

CONFIG_DIR = os.path.join(os.environ.get("APPDATA", os.path.expanduser("~")), "dictation_hotkey")
CONFIG_FILE = os.path.join(CONFIG_DIR, "config.json")

DEFAULTS = {
    "api_key": "",
    "hotkey_copilot": False,
    "hotkey_win_h": True,
    "hotkey_custom": "",
    "language": "",
    "start_with_windows": False,
}

STARTUP_DIR = os.path.join(
    os.environ.get("APPDATA", os.path.expanduser("~")),
    "Microsoft", "Windows", "Start Menu", "Programs", "Startup",
)
SHORTCUT_PATH = os.path.join(STARTUP_DIR, "Dictation Hotkey.lnk")


def load() -> dict:
    """Load config from disk, returning defaults for missing keys."""
    config = dict(DEFAULTS)
    try:
        with open(CONFIG_FILE, "r", encoding="utf-8") as f:
            config.update(json.load(f))
    except (FileNotFoundError, json.JSONDecodeError):
        pass
    return config


def get_hotkey_combos(cfg: dict) -> list[str]:
    """Convert config booleans/string into a list of hotkey combo strings."""
    combos = []
    if cfg.get("hotkey_copilot"):
        combos.extend(["Win+C", "Win+Shift+F23"])
    if cfg.get("hotkey_win_h"):
        combos.append("Win+H")
    custom = cfg.get("hotkey_custom", "").strip()
    if custom:
        combos.append(custom)
    return combos


def save(config: dict):
    """Save config to disk."""
    os.makedirs(CONFIG_DIR, exist_ok=True)
    with open(CONFIG_FILE, "w", encoding="utf-8") as f:
        json.dump(config, f, indent=2)


def set_startup_shortcut(enable: bool):
    """Create or remove a .lnk shortcut in the Windows Startup folder."""
    if enable:
        if getattr(sys, "frozen", False):
            target = sys.executable
            arguments = ""
        else:
            # Use pythonw.exe to avoid a console window on startup
            python = sys.executable
            if python.endswith("python.exe"):
                python = python[:-len("python.exe")] + "pythonw.exe"
            target = python
            script = os.path.abspath(os.path.join(os.path.dirname(__file__), "main.py"))
            arguments = f'"{script}"'

        work_dir = os.path.dirname(target)

        def _ps(s):
            return s.replace("'", "''")

        ps_cmd = (
            "$ws = New-Object -ComObject WScript.Shell; "
            f"$s = $ws.CreateShortcut('{_ps(SHORTCUT_PATH)}'); "
            f"$s.TargetPath = '{_ps(target)}'; "
            f"$s.Arguments = '{_ps(arguments)}'; "
            f"$s.WorkingDirectory = '{_ps(work_dir)}'; "
            "$s.Save()"
        )
        subprocess.run(
            ["powershell", "-NoProfile", "-Command", ps_cmd],
            capture_output=True,
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
    else:
        if os.path.exists(SHORTCUT_PATH):
            os.remove(SHORTCUT_PATH)
