//! Transport-independent request construction; tested without Windows or credentials.
use crate::{protocol::MAX_AUDIO_FRAME, session::MAX_TRANSCRIPT_BYTES};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::io;
pub const DEFAULT_MODEL: &str = "voxtral-mini-transcribe-realtime-2602";
pub const DEFAULT_BATCH_MODEL: &str = "voxtral-mini-latest";
pub const DEFAULT_URL: &str = "wss://api.mistral.ai";
pub fn valid_model(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= 128
        && model
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}
pub fn endpoint(base: &str, model: &str) -> io::Result<(String, u16, String)> {
    let base = base
        .strip_prefix("wss://")
        .or_else(|| base.strip_prefix("https://"))
        .ok_or_else(|| io::Error::other("realtime URL must use wss:// or https://"))?;
    if !valid_model(model) {
        return Err(io::Error::other("invalid realtime model"));
    }
    let (hostport, prefix) = base.split_once('/').unwrap_or((base, ""));
    let (host, port) = if let Some((host, port)) = hostport.rsplit_once(':') {
        (
            host,
            port.parse::<u16>()
                .map_err(|_| io::Error::other("invalid port"))?,
        )
    } else {
        (hostport, 443)
    };
    if host.is_empty()
        || port == 0
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
        || prefix.contains("..")
        || !prefix
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_/%.".contains(&b))
    {
        return Err(io::Error::other(
            "invalid realtime URL (use a DNS hostname and optional path)",
        ));
    }
    let prefix = prefix.trim_matches('/');
    let path = if prefix.is_empty() {
        String::new()
    } else {
        format!("/{prefix}")
    };
    Ok((
        host.to_owned(),
        port,
        format!("{path}/v1/audio/transcriptions/realtime?model={model}"),
    ))
}
pub fn append(pcm: &[u8]) -> io::Result<String> {
    if pcm.len() > MAX_AUDIO_FRAME || !pcm.len().is_multiple_of(2) {
        return Err(io::Error::other("invalid PCM frame length"));
    }
    Ok(format!(
        "{{\"type\":\"input_audio.append\",\"audio\":\"{}\"}}",
        STANDARD.encode(pcm)
    ))
}
pub struct Multipart {
    pub prefix: String,
    pub suffix: String,
    pub total: u32,
}
impl Multipart {
    pub fn new(boundary: &str, model: &str, file_len: u64) -> io::Result<Self> {
        if !valid_model(model)
            || boundary.is_empty()
            || boundary.len() > 70
            || !boundary
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(io::Error::other("invalid multipart fields"));
        }
        let prefix = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\n{model}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"recording.wav\"\r\nContent-Type: audio/wav\r\n\r\n");
        let suffix = format!("\r\n--{boundary}--\r\n");
        let total = file_len
            .checked_add(prefix.len() as u64)
            .and_then(|n| n.checked_add(suffix.len() as u64))
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| io::Error::other("upload too large"))?;
        Ok(Self {
            prefix,
            suffix,
            total,
        })
    }
}
pub fn batch_text(body: &[u8]) -> io::Result<String> {
    if body.len() > MAX_TRANSCRIPT_BYTES + 64 * 1024 {
        return Err(io::Error::other("batch response exceeds limit"));
    }
    let json: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| io::Error::other("invalid batch response"))?;
    let text = json
        .get("text")
        .and_then(|s| s.as_str())
        .ok_or_else(|| io::Error::other("missing transcript"))?;
    if text.len() > MAX_TRANSCRIPT_BYTES {
        return Err(io::Error::other("transcript limit reached"));
    }
    Ok(text.trim_start().to_owned())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secure_endpoint() {
        for bad in [
            "ws://localhost",
            "wss://user@api.mistral.ai",
            "https://example.org/?a=1",
            "wss://example.org/../x",
            "wss://example.org:0",
        ] {
            assert!(endpoint(bad, "model").is_err());
        }
        assert_eq!(
            endpoint(DEFAULT_URL, DEFAULT_MODEL).unwrap().2,
            format!("/v1/audio/transcriptions/realtime?model={DEFAULT_MODEL}")
        );
        assert!(endpoint("wss://test.org:8443/proxy/", "model")
            .unwrap()
            .2
            .starts_with("/proxy/v1/"));
    }
    #[test]
    fn audio_frame() {
        assert_eq!(
            append(&[0, 1, 2, 3]).unwrap(),
            r#"{"type":"input_audio.append","audio":"AAECAw=="}"#
        );
        assert!(append(&[0]).is_err());
        assert!(append(&[0; 3202]).is_err());
    }
    #[test]
    fn multipart_lengths_and_injection() {
        let m = Multipart::new("safe-boundary", "model", 44).unwrap();
        assert_eq!(m.total as usize, m.prefix.len() + 44 + m.suffix.len());
        assert!(m.prefix.contains("filename=\"recording.wav\""));
        assert!(Multipart::new("bad\r\n", "model", 0).is_err());
        assert!(Multipart::new("good", "bad\r\nmodel", 0).is_err());
        assert!(Multipart::new("good", "model", u64::MAX).is_err());
    }
    #[test]
    fn bounded_batch_reply() {
        assert_eq!(batch_text(br#"{"text":" hello"}"#).unwrap(), "hello");
        assert!(batch_text(b"{}").is_err());
        assert!(batch_text(&vec![b'x'; MAX_TRANSCRIPT_BYTES + 65537]).is_err());
    }
}
