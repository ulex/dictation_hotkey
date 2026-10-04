"""Check sanitized structural fixtures against the pinned SDK, without network requests.

Run in an isolated environment: pip install 'mistralai==1.12.4'; python tools/verify_sdk_protocol.py
The SDK is a reference tool only, not a native runtime dependency.
"""
import importlib.metadata
import json
from pathlib import Path
from mistralai.models import (
    AudioFormat, RealtimeTranscriptionSessionUpdateMessage,
    RealtimeTranscriptionSessionUpdatePayload, RealtimeTranscriptionInputAudioAppend,
    RealtimeTranscriptionInputAudioFlush, RealtimeTranscriptionInputAudioEnd,
    TranscriptionStreamTextDelta,
)

assert importlib.metadata.version("mistralai") == "1.12.4"
fixture = json.loads((Path(__file__).resolve().parent.parent / "native/tests/fixtures/sdk-1.12.4.json").read_text(encoding="utf-8"))
messages = {
    "session_update": RealtimeTranscriptionSessionUpdateMessage(session=RealtimeTranscriptionSessionUpdatePayload(audio_format=AudioFormat(encoding="pcm_s16le", sample_rate=16000))),
    "append": RealtimeTranscriptionInputAudioAppend(audio="AAECAw=="),
    "flush": RealtimeTranscriptionInputAudioFlush(),
    "end": RealtimeTranscriptionInputAudioEnd(),
    "delta": TranscriptionStreamTextDelta(text="Fixture 😀"),
}
for key, message in messages.items():
    assert json.loads(message.model_dump_json()) == fixture[key], key
print("Five sanitized protocol fixtures match mistralai==1.12.4; no API traffic sent.")
