import json
import os

CONFIG_DIR = os.path.join(os.environ.get("APPDATA", os.path.expanduser("~")), "dictation_hotkey")
CONFIG_FILE = os.path.join(CONFIG_DIR, "config.json")

DEFAULTS = {
    "api_key": "",
    "hotkey_copilot": False,
    "hotkey_win_h": True,
    "hotkey_custom": "",
    "language": "",
}


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
