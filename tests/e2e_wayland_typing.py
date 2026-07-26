#!/usr/bin/env python3
"""End-to-end Wayland test for the Linux typing backend.

Starts the minimal test compositor (tests/wayland_compositor.py), a Qt
text editor client (tests/qt_editor_client.py), then uses the app's real
typing_linux backend to type text (including Unicode) and backspaces into
the focused editor, asserting the received text.

Requires: pywayland, PySide6, libwayland-client and libxkbcommon.

Run:  python tests/e2e_wayland_typing.py
"""

import os
import subprocess
import sys
import tempfile
import time

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, REPO_ROOT)

SOCKET = "dictation-e2e"
TEXT = "Hello ✓ éàü 123"
BACKSPACES = 3
EXPECTED = TEXT[:-BACKSPACES]


def wait_for(proc, token, timeout=15):
    """Wait until `token` appears on proc's stdout."""
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
    raise RuntimeError(f"Timed out waiting for {token!r} from {proc.args[1]}")


def wait_for_file(path, expected, timeout=15):
    start = time.time()
    last = None
    while time.time() - start < timeout:
        try:
            with open(path, "r", encoding="utf-8") as f:
                last = f.read()
            if last == expected:
                return True
        except OSError:
            pass
        time.sleep(0.1)
    raise AssertionError(f"Editor content mismatch:\n  expected {expected!r}\n  got      {last!r}")


def main():
    runtime_dir = os.environ.get("XDG_RUNTIME_DIR") or tempfile.mkdtemp(
        prefix="dictation-xdg-"
    )
    os.environ["XDG_RUNTIME_DIR"] = runtime_dir
    os.environ["WAYLAND_DISPLAY"] = SOCKET
    os.environ["QT_QPA_PLATFORM"] = "wayland"

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
        # Give the editor a moment to map and gain keyboard focus
        time.sleep(1.0)

        # Use the app's real typing backend
        import typing_linux

        typing_linux.type_text(TEXT)
        typing_linux.press_backspace(BACKSPACES)

        wait_for_file(out_file, EXPECTED)
        print(f"PASS: editor received {EXPECTED!r} via Wayland virtual keyboard")
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
        if os.path.exists(out_file):
            os.unlink(out_file)


if __name__ == "__main__":
    main()
