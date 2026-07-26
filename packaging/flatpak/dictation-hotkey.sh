#!/bin/sh
# Flatpak entry point: run the app installed under /app/share/dictation-hotkey.
exec python3 /app/share/dictation-hotkey/main.py "$@"
