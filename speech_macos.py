"""Native macOS speech-to-text provider.

Uses the Speech framework (SFSpeechRecognizer) with an AVAudioEngine input
tap for streaming, on-device recognition — no API key or network required.
Partial results are diffed against the previously emitted text so revisions
are sent as backspaces + replacement text, keeping the real-time typing UX.

Requires:
  * Speech Recognition permission (prompted on first use)
  * Microphone permission (prompted on first use)
  * Accessibility permission (for typing + hotkey, same as the rest of the app)
"""

from PySide6.QtCore import QObject, Signal

import AVFoundation
import Speech


class NativeSpeechWorker(QObject):
    """Streams mic audio to SFSpeechRecognizer and emits text deltas."""

    text_delta = Signal(str)
    backspaces = Signal(int)
    status_changed = Signal(str)
    error = Signal(str)
    finished = Signal()

    needs_api_key = False
    needs_audio_queue = False  # captures audio itself via AVAudioEngine

    def __init__(self, parent=None):
        super().__init__(parent)
        self._running = False
        self._engine = None
        self._request = None
        self._task = None
        self._typed = ""  # text emitted so far for the current session

    @property
    def is_running(self) -> bool:
        return self._running

    def start(self, api_key: str = "", audio_queue=None, language: str = ""):
        """Start a streaming recognition session.

        `api_key` and `audio_queue` are accepted for interface compatibility
        with TranscriptionWorker and ignored.
        """
        self._typed = ""

        auth = Speech.SFSpeechRecognizer.authorizationStatus()
        if auth == Speech.SFSpeechRecognizerAuthorizationStatusNotDetermined:
            Speech.SFSpeechRecognizer.requestAuthorization_(lambda status: None)
            self.error.emit(
                "Speech recognition permission requested — grant it in the "
                "system prompt, then press the hotkey again."
            )
            self.finished.emit()
            return
        if auth != Speech.SFSpeechRecognizerAuthorizationStatusAuthorized:
            self.error.emit(
                "Speech recognition not authorized. Enable it in System "
                "Settings -> Privacy & Security -> Speech Recognition."
            )
            self.finished.emit()
            return

        mic_auth = AVFoundation.AVCaptureDevice.authorizationStatusForMediaType_(
            AVFoundation.AVMediaTypeAudio
        )
        if mic_auth == AVFoundation.AVAuthorizationStatusNotDetermined:
            AVFoundation.AVCaptureDevice.requestAccessForMediaType_completionHandler_(
                AVFoundation.AVMediaTypeAudio, lambda granted: None
            )
            self.error.emit(
                "Microphone permission requested — grant it in the system "
                "prompt, then press the hotkey again."
            )
            self.finished.emit()
            return
        if mic_auth != AVFoundation.AVAuthorizationStatusAuthorized:
            self.error.emit(
                "Microphone access not authorized. Enable it in System "
                "Settings -> Privacy & Security -> Microphone."
            )
            self.finished.emit()
            return

        if language:
            from Foundation import NSLocale

            locale = NSLocale.alloc().initWithLocaleIdentifier_(language)
            recognizer = Speech.SFSpeechRecognizer.alloc().initWithLocale_(locale)
        else:
            recognizer = Speech.SFSpeechRecognizer.alloc().init()
        if recognizer is None or not recognizer.isAvailable():
            self.error.emit("Speech recognizer is not available for this locale.")
            self.finished.emit()
            return

        self._request = Speech.SFSpeechAudioBufferRecognitionRequest.alloc().init()
        self._request.setShouldReportPartialResults_(True)
        if recognizer.supportsOnDeviceRecognition():
            self._request.setRequiresOnDeviceRecognition_(True)

        self._engine = AVFoundation.AVAudioEngine.alloc().init()
        input_node = self._engine.inputNode()
        tap_format = input_node.outputFormatForBus_(0)

        request = self._request

        def tap_callback(buffer, when):
            request.appendAudioPCMBuffer_(buffer)

        input_node.installTapOnBus_bufferSize_format_block_(
            0, 4096, tap_format, tap_callback
        )

        self.status_changed.emit("connecting")
        self._task = recognizer.recognitionTaskWithRequest_resultHandler_(
            self._request, self._on_result
        )

        self._engine.prepare()
        ok, err = self._engine.startAndReturnError_(None)
        if not ok:
            msg = err.localizedDescription() if err is not None else "unknown error"
            self._cleanup()
            self.error.emit(f"Audio engine failed to start: {msg}")
            self.finished.emit()
            return

        self._running = True
        self.status_changed.emit("listening")

    def stop(self):
        """Stop the recognition session."""
        was_running = self._running
        self._running = False
        self._cleanup()
        if was_running:
            self.finished.emit()

    def _cleanup(self):
        if self._engine is not None:
            try:
                self._engine.inputNode().removeTapOnBus_(0)
                self._engine.stop()
            except Exception:
                pass
            self._engine = None
        if self._request is not None:
            try:
                self._request.endAudio()
            except Exception:
                pass
            self._request = None
        if self._task is not None:
            try:
                self._task.cancel()
            except Exception:
                pass
            self._task = None

    def _on_result(self, result, error):
        """Recognition task callback (partial and final results)."""
        if not self._running:
            return
        if error is not None:
            self._running = False
            self.error.emit(str(error.localizedDescription()))
            self.finished.emit()
            return
        if result is None:
            return
        text = str(result.bestTranscription().formattedString())
        self._emit_text(text)

    def _emit_text(self, new_text: str):
        """Diff against previously emitted text; send backspaces + new suffix."""
        old = self._typed
        common = 0
        limit = min(len(old), len(new_text))
        while common < limit and old[common] == new_text[common]:
            common += 1
        removed = len(old) - common
        if removed:
            self.backspaces.emit(removed)
        delta = new_text[common:]
        if delta:
            self.text_delta.emit(delta)
        self._typed = new_text
