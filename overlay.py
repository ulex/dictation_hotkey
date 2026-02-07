from PySide6.QtCore import Qt, QTimer, QPropertyAnimation, QEasingCurve
from PySide6.QtGui import QColor, QPainter, QBrush, QFont
from PySide6.QtWidgets import QWidget, QLabel, QHBoxLayout, QApplication, QGraphicsOpacityEffect


class RecordingDot(QWidget):
    """Pulsing red dot indicator."""

    def __init__(self, parent=None):
        super().__init__(parent)
        self.setFixedSize(12, 12)
        self._opacity_effect = QGraphicsOpacityEffect(self)
        self.setGraphicsEffect(self._opacity_effect)
        self._animation = QPropertyAnimation(self._opacity_effect, b"opacity")
        self._animation.setDuration(800)
        self._animation.setStartValue(1.0)
        self._animation.setEndValue(0.3)
        self._animation.setEasingCurve(QEasingCurve.Type.InOutSine)
        self._animation.setLoopCount(-1)
        # Reverse each cycle
        self._animation.finished.connect(self._animation.start)

    def paintEvent(self, event):
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        p.setBrush(QBrush(QColor(220, 38, 38)))
        p.setPen(Qt.PenStyle.NoPen)
        p.drawEllipse(1, 1, 10, 10)
        p.end()

    def start(self):
        self.show()
        self._animation.start()

    def stop(self):
        self._animation.stop()
        self._opacity_effect.setOpacity(1.0)
        self.hide()


class OverlayWidget(QWidget):
    """Small always-on-top translucent status overlay at the top-center of the screen."""

    def __init__(self):
        super().__init__()
        self.setWindowFlags(
            Qt.WindowType.FramelessWindowHint
            | Qt.WindowType.WindowStaysOnTopHint
            | Qt.WindowType.Tool
            | Qt.WindowType.WindowTransparentForInput
        )
        self.setAttribute(Qt.WidgetAttribute.WA_TranslucentBackground)
        self.setFixedSize(220, 44)

        layout = QHBoxLayout(self)
        layout.setContentsMargins(16, 8, 16, 8)
        layout.setSpacing(8)

        self._dot = RecordingDot(self)
        self._dot.hide()
        layout.addWidget(self._dot)

        self._label = QLabel("Ready")
        self._label.setFont(QFont("Segoe UI", 11, QFont.Weight.DemiBold))
        self._label.setStyleSheet("color: white;")
        layout.addWidget(self._label)

        self._hide_timer = QTimer(self)
        self._hide_timer.setSingleShot(True)
        self._hide_timer.timeout.connect(self.hide)

    def paintEvent(self, event):
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing)
        p.setBrush(QBrush(QColor(30, 30, 30, 210)))
        p.setPen(Qt.PenStyle.NoPen)
        p.drawRoundedRect(self.rect(), 10, 10)
        p.end()

    def _center_on_screen(self):
        screen = QApplication.primaryScreen().geometry()
        x = (screen.width() - self.width()) // 2
        self.move(x, 24)

    def show_status(self, text: str, recording: bool = False, auto_hide_ms: int = 0):
        """Update and show the overlay."""
        self._hide_timer.stop()
        self._label.setText(text)
        if recording:
            self._dot.start()
        else:
            self._dot.stop()
        self._center_on_screen()
        self.show()
        if auto_hide_ms > 0:
            self._hide_timer.start(auto_hide_ms)
