//! Byte- and entry-bounded diagnostics. Only timings/status codes are logged; never transcripts,
//! credentials, raw audio, request headers or provider response bodies.
use std::collections::VecDeque;
pub const MAX_LOG_BYTES: usize = 256 * 1024;
pub const MAX_ENTRY_BYTES: usize = 1024;
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
