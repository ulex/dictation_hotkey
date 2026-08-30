import threading
from collections import deque

from PySide6.QtCore import QObject, Signal

_buffer: deque[str] = deque(maxlen=2000)
_lock = threading.Lock()
_emitter: "LogEmitter | None" = None


class LogEmitter(QObject):
    new_line = Signal(str)


def init_emitter() -> "LogEmitter":
    """Create the emitter on the main thread before any background threads log."""
    global _emitter
    _emitter = LogEmitter()
    return _emitter


def log(msg: str) -> None:
    with _lock:
        _buffer.append(msg)
    print(msg, flush=True)
    if _emitter is not None:
        _emitter.new_line.emit(msg)


def get_all() -> list[str]:
    with _lock:
        return list(_buffer)


def clear() -> None:
    with _lock:
        _buffer.clear()
