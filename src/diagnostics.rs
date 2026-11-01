//! Byte- and entry-bounded diagnostics. Only timings/status codes are logged; never transcripts,
//! credentials, raw audio, request headers or provider response bodies.
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};
pub const MAX_LOG_BYTES: usize = 256 * 1024;
pub const MAX_ENTRY_BYTES: usize = 1024;

/// Send a bounded, non-sensitive operational message to the native Logs UI.
pub fn log(message: &str) {
    #[cfg(windows)]
    crate::logs_ui::log(message);
    #[cfg(target_os = "macos")]
    if let Ok(message) = crate::macos_bridge::string(message) {
        unsafe { crate::macos_bridge::dh_ui_log(message.as_ptr()) };
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = message;
}

#[derive(Clone, Copy)]
#[repr(usize)]
pub enum BatchPhase {
    Setup = 0,
    MultipartStage = 1,
    Upload = 2,
    ServerWait = 3,
    ResponseBody = 4,
    ResponseParse = 5,
    NetworkTotal = 6,
    Connection = 7,
}

/// Collects only batch/offline timings and emits one summary on every exit path.
pub struct BatchTimingLog {
    enabled: bool,
    started: Instant,
    phases: [Option<Duration>; 8],
    audio_bytes: Option<u64>,
    succeeded: bool,
}
impl BatchTimingLog {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            started: Instant::now(),
            phases: [None; 8],
            audio_bytes: None,
            succeeded: false,
        }
    }
    pub fn record(&mut self, phase: BatchPhase, duration: Duration) {
        if self.enabled {
            self.phases[phase as usize] = Some(duration);
        }
    }
    pub fn set_audio_bytes(&mut self, bytes: u64) {
        if self.enabled {
            self.audio_bytes = Some(bytes);
        }
    }
    pub fn mark_succeeded(&mut self) {
        self.succeeded = true;
    }
}
impl Drop for BatchTimingLog {
    fn drop(&mut self) {
        if !self.enabled {
            return;
        }
        let phase_ms = |phase: BatchPhase| {
            self.phases[phase as usize]
                .map(|duration| duration.as_millis().to_string())
                .unwrap_or_else(|| "n/a".into())
        };
        log(&format!(
            "Offline batch timing: result={} total_ms={} audio_bytes={} setup_ms={} multipart_stage_ms={} connection_ms={} upload_ms={} upload_to_response_ms={} response_body_ms={} response_parse_ms={} network_total_ms={}",
            if self.succeeded { "success" } else { "failed" },
            self.started.elapsed().as_millis(),
            self.audio_bytes.map_or_else(|| "n/a".into(), |bytes| bytes.to_string()),
            phase_ms(BatchPhase::Setup),
            phase_ms(BatchPhase::MultipartStage),
            phase_ms(BatchPhase::Connection),
            phase_ms(BatchPhase::Upload),
            phase_ms(BatchPhase::ServerWait),
            phase_ms(BatchPhase::ResponseBody),
            phase_ms(BatchPhase::ResponseParse),
            phase_ms(BatchPhase::NetworkTotal),
        ));
    }
}
#[derive(Default)]
pub struct Ring {
    entries: VecDeque<String>,
    bytes: usize,
}
impl Ring {
    pub fn push(&mut self, message: &str) {
        let mut end = message.len().min(MAX_ENTRY_BYTES);
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        let line = format!("{}\r\n", &message[..end]);
        while self.bytes + line.len() > MAX_LOG_BYTES || self.entries.len() >= 2000 {
            if let Some(old) = self.entries.pop_front() {
                self.bytes -= old.len();
            } else {
                break;
            }
        }
        self.bytes += line.len();
        self.entries.push_back(line);
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }
    pub fn snapshot(&self) -> String {
        self.entries.iter().map(String::as_str).collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_and_unicode() {
        let mut r = Ring::default();
        for _ in 0..10000 {
            r.push(&"😀".repeat(2000));
        }
        assert!(r.bytes <= MAX_LOG_BYTES);
        assert!(r.entries.len() <= 2000);
        assert!(r.entries.iter().all(|s| s.len() <= MAX_ENTRY_BYTES + 2));
        r.clear();
        assert!(r.snapshot().is_empty());
    }
}
