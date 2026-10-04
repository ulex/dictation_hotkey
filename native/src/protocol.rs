//! Verified against mistralai 1.12.4 source; not a live service conformance test.
use serde_json::{json, Value};
use std::io;

pub const MAX_EVENT_BYTES: usize = 64 * 1024;
pub const MAX_AUDIO_FRAME: usize = 3200;
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    Created,
    Delta(String),
    Done,
    Error(String),
    Unknown,
}

pub fn parse_event(raw: &[u8]) -> io::Result<Event> {
    if raw.len() > MAX_EVENT_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "event too large",
        ));
    }
    let value: Value =
        serde_json::from_slice(raw).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if !value.is_object() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "event is not an object",
        ));
    }
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing event type"))?;
    Ok(match kind {
        "session.created" => Event::Created,
        "transcription.text.delta" => Event::Delta(
            value
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing delta text"))?
                .to_owned(),
        ),
        "transcription.done" => Event::Done,
        "error" => Event::Error(
            value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("service error")
                .chars()
                .take(512)
                .collect(),
        ),
        _ => Event::Unknown,
    })
}

#[derive(Default)]
pub struct Assembler {
    bytes: Vec<u8>,
}
impl Assembler {
    pub fn push(&mut self, fragment: &[u8], complete: bool) -> io::Result<Option<Event>> {
        if self
            .bytes
            .len()
            .checked_add(fragment.len())
            .is_none_or(|len| len > MAX_EVENT_BYTES)
        {
            self.bytes.clear();
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "event too large",
            ));
        }
        self.bytes.extend_from_slice(fragment);
        if complete {
            let result = parse_event(&self.bytes);
            self.bytes.clear();
            result.map(Some)
        } else {
            Ok(None)
        }
    }
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

pub fn session_update() -> Value {
    json!({"type": "session.update", "session": {"audio_format": {"encoding": "pcm_s16le", "sample_rate": 16000}}})
}
pub fn flush() -> Value {
    json!({"type": "input_audio.flush"})
}
pub fn end() -> Value {
    json!({"type": "input_audio.end"})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragmented_utf8_and_hostile_frames() {
        let raw = r#"{"type":"transcription.text.delta","text":"😀"}"#.as_bytes();
        for split in 1..raw.len() {
            let mut a = Assembler::default();
            assert_eq!(a.push(&raw[..split], false).unwrap(), None);
            assert_eq!(
                a.push(&raw[split..], true).unwrap(),
                Some(Event::Delta("😀".into()))
            );
            assert!(a.is_empty());
        }
        let mut a = Assembler::default();
        assert!(a.push(&vec![b' '; MAX_EVENT_BYTES], false).is_ok());
        assert!(a.push(b" ", true).is_err());
        assert!(parse_event(b"[]").is_err());
        assert!(parse_event(b"{}").is_err());
        assert!(parse_event(b"{bad}").is_err());
    }
    #[test]
    fn sdk_equivalent_fixture() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/sdk-1.12.4.json")).unwrap();
        assert_eq!(session_update(), fixture["session_update"]);
        assert_eq!(flush(), fixture["flush"]);
        assert_eq!(end(), fixture["end"]);
        assert_eq!(
            serde_json::from_str::<Value>(&crate::wire::append(&[0, 1, 2, 3]).unwrap()).unwrap(),
            fixture["append"]
        );
        assert_eq!(
            parse_event(fixture["delta"].to_string().as_bytes()).unwrap(),
            Event::Delta("Fixture 😀".into())
        );
    }
    #[test]
    fn bounded_events() {
        assert_eq!(
            parse_event(r#"{"type":"transcription.text.delta","text":"😀"}"#.as_bytes()).unwrap(),
            Event::Delta("😀".into())
        );
        assert_eq!(
            parse_event(br#"{"type":"transcription.done"}"#).unwrap(),
            Event::Done
        );
        assert_eq!(
            parse_event(br#"{"type":"future.event"}"#).unwrap(),
            Event::Unknown
        );
        assert!(parse_event(&vec![b' '; MAX_EVENT_BYTES + 1]).is_err());
        assert!(parse_event(br#"{"type":"transcription.text.delta"}"#).is_err());
        assert_eq!(flush()["type"], "input_audio.flush");
    }
}
