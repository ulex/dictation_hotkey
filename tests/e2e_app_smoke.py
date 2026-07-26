#!/usr/bin/env python3
"""End-to-end app smoke test on Wayland.

Starts the test compositor and a Qt editor client, then instantiates the
real main.App inside a Wayland session and feeds it a fake transcription
delta, verifying that the text is typed into the focused editor.

Exercises: config, tray, overlay, hotkey startup (without input device
permissions), and the full typing path on Wayland.

Run:  python tests/e2e_app_smoke.py
"""

import json
import os
import subprocess
import sys
import tempfile
import time

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, REPO_ROOT)

SOCKET = "dictation-smoke"
TEXT = "Smoke ✓ test"

# Point config at a temp dir with a pre-seeded API key *before* importing
# the app modules (config paths are computed at import time).
config_home = tempfile.mkdtemp(prefix="dictation-config-")
os.environ["XDG_CONFIG_HOME"] = config_home
os.environ["WAYLAND_DISPLAY"] = SOCKET
os.environ["QT_QPA_PLATFORM"] = "wayland"
runtime_dir = os.environ.get("XDG_RUNTIME_DIR") or tempfile.mkdtemp(
    prefix="dictation-xdg-"
)
os.environ["XDG_RUNTIME_DIR"] = runtime_dir

import config  # noqa: E402

os.makedirs(config.CONFIG_DIR, exist_ok=True)
with open(config.CONFIG_FILE, "w", encoding="utf-8") as f:
    json.dump({"api_key": "dummy-key-for-smoke-test"}, f)


def wait_for(proc, token, timeout=15):
    start = time.time()
    line = b""
    while time.time() - start < timeout:
        ch = proc.stdout.read(1)
        if not ch:
            break
        line += ch
        if token.encode() in line:
            return True
        if ch == b"\n":
            line = b""
    raise RuntimeError(f"Timed out waiting for {token!r}")


def main():
    out_file = tempfile.mktemp(prefix="dictation-editor-out-", suffix=".txt")

    compositor = subprocess.Popen(
        [sys.executable, os.path.join(REPO_ROOT, "tests", "wayland_compositor.py"), SOCKET],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    editor = None
    try:
        wait_for(compositor, "READY")

        env = dict(os.environ)
        env["TEST_OUT"] = out_file
        env["TEST_TIMEOUT"] = "60"
        editor = subprocess.Popen(
            [sys.executable, os.path.join(REPO_ROOT, "tests", "qt_editor_client.py")],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            env=env,
        )
        wait_for(editor, "EDITOR_READY")
        time.sleep(1.0)

        from PySide6.QtWidgets import QApplication

        app = QApplication([])
        app.setQuitOnLastWindowClosed(False)

        import main as app_main

        controller = app_main.App()
        assert controller._hotkey is not None
        print("App initialised (tray, overlay, hotkey, worker)")

        # Simulate a transcription delta arriving
        controller._on_text_delta(TEXT)
        app.processEvents()

        # Wait for the editor to record the text
        start = time.time()
        last = None
        while time.time() - start < 15:
            app.processEvents()
            try:
                with open(out_file, "r", encoding="utf-8") as f:
                    last = f.read()
                if last == TEXT:
                    print(f"PASS: App typed {TEXT!r} into the Wayland-focused editor")
                    return
            except OSError:
                pass
            time.sleep(0.1)
        raise AssertionError(f"Editor content mismatch: expected {TEXT!r}, got {last!r}")
    finally:
        if editor is not None:
            editor.terminate()
        compositor.terminate()
        for proc in (editor, compositor):
            if proc is not None:
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.kill()
        socket_path = os.path.join(runtime_dir, SOCKET)
        if os.path.exists(socket_path):
            os.unlink(socket_path)


if __name__ == "__main__":
    main()
