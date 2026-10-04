//! Pure state machine; all worker events carry both session and operation IDs.
pub const MAX_TRANSCRIPT_BYTES: usize = 256 * 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Starting,
    RecordingRealtime,
    RecordingBatch,
    RecordingFallback,
    DrainingRealtime,
    SubmittingBatch,
    Failed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ticket {
    pub session: u64,
    pub operation: u64,
}
#[derive(Debug)]
pub struct Controller {
    pub phase: Phase,
    pub ticket: Ticket,
    pub text: String,
    pub last_text: String,
    pub injected_prefix: bool,
    pub clipboard_only: bool,
    pub problem: Option<&'static str>,
}
impl Default for Controller {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            ticket: Ticket {
                session: 0,
                operation: 0,
            },
            text: String::new(),
            last_text: String::new(),
            injected_prefix: false,
            clipboard_only: false,
            problem: None,
        }
    }
}
impl Controller {
    pub fn start(&mut self, batch: bool, clipboard_only: bool) -> Option<Ticket> {
        if self.phase != Phase::Idle {
            return None;
        }
        self.ticket.session = self.ticket.session.wrapping_add(1);
        self.ticket.operation = self.ticket.operation.wrapping_add(1);
        self.phase = Phase::Starting;
        self.text.clear();
        self.problem = None;
        self.injected_prefix = false;
        self.clipboard_only = clipboard_only;
        self.phase = if batch {
            Phase::RecordingBatch
        } else {
            Phase::RecordingRealtime
        };
        Some(self.ticket)
    }
    pub fn stop(&mut self) -> bool {
        self.phase = match self.phase {
            Phase::RecordingRealtime => Phase::DrainingRealtime,
            Phase::RecordingBatch | Phase::RecordingFallback => Phase::SubmittingBatch,
            _ => return false,
        };
        true
    }
    pub fn realtime_failure(&mut self, ticket: Ticket) {
        if self.ticket != ticket {
            return;
        }
        match self.phase {
            Phase::RecordingRealtime => {
                self.phase = Phase::RecordingFallback;
                self.ticket.operation = self.ticket.operation.wrapping_add(1);
            }
            Phase::DrainingRealtime => {
                self.phase = Phase::SubmittingBatch;
                self.ticket.operation = self.ticket.operation.wrapping_add(1);
            }
            _ => (),
        }
    }
    pub fn delta(&mut self, ticket: Ticket, text: &str) -> bool {
        if self.ticket != ticket
            || !matches!(
                self.phase,
                Phase::RecordingRealtime | Phase::DrainingRealtime | Phase::SubmittingBatch
            )
        {
            return false;
        }
        let text = if self.text.is_empty() {
            text.trim_start()
        } else {
            text
        };
        if self
            .text
            .len()
            .checked_add(text.len())
            .is_none_or(|n| n > MAX_TRANSCRIPT_BYTES)
        {
            self.problem = Some("transcript limit reached");
            self.phase = Phase::Failed;
            return false;
        }
        self.text.push_str(text);
        !text.is_empty() && !self.clipboard_only && self.phase != Phase::SubmittingBatch
    }
    pub fn output_succeeded(&mut self, ticket: Ticket) {
        if ticket == self.ticket {
            self.injected_prefix = true;
        }
    }
    pub fn batch_result(&mut self, ticket: Ticket, text: String) -> bool {
        if ticket != self.ticket || self.phase != Phase::SubmittingBatch {
            return false;
        }
        if text.len() > MAX_TRANSCRIPT_BYTES {
            self.phase = Phase::Failed;
            self.problem = Some("transcript limit reached");
            return false;
        }
        self.text = text;
        // Do not insert a full batch transcript after a partial realtime prefix.
        !self.injected_prefix && !self.clipboard_only && !self.text.is_empty()
    }
    pub fn finish(&mut self, ticket: Ticket) {
        if self.ticket != ticket
            || !matches!(self.phase, Phase::DrainingRealtime | Phase::SubmittingBatch)
        {
            return;
        }
        if !self.text.is_empty() {
            self.last_text.clone_from(&self.text);
        }
        self.phase = Phase::Idle;
    }
    pub fn fail(&mut self, ticket: Ticket, reason: &'static str) {
        if self.ticket == ticket {
            self.problem = Some(reason);
            self.phase = Phase::Failed;
        }
    }
    pub fn reset(&mut self) {
        if self.phase == Phase::Failed {
            self.phase = Phase::Idle;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failure_completion_cannot_erase_error() {
        let mut c = Controller::default();
        let ticket = c.start(true, false).unwrap();
        assert!(c.stop());
        c.fail(ticket, "batch failed");
        c.finish(ticket);
        assert_eq!(c.phase, Phase::Failed);
        assert_eq!(c.problem, Some("batch failed"));
        c.reset();
        let next = c.start(false, false).unwrap();
        c.fail(ticket, "obsolete error");
        c.finish(ticket);
        assert_eq!(c.phase, Phase::RecordingRealtime);
        assert_ne!(next, ticket);
    }
    #[test]
    fn repeated_sessions_with_fallback_and_empty_results() {
        let mut c = Controller::default();
        for _ in 0..100 {
            let ticket = c.start(false, true).unwrap();
            assert!(!c.delta(ticket, "  "));
            assert!(!c.delta(ticket, " word"));
            assert_eq!(c.text, "word");
            c.realtime_failure(ticket);
            assert!(!c.delta(ticket, "old transport text"));
            assert!(c.stop());
            assert!(!c.stop());
            let next = c.ticket;
            assert!(!c.batch_result(next, "complete text".into()));
            c.finish(next);
            assert_eq!(c.last_text, "complete text");
            let empty = c.start(true, true).unwrap();
            assert!(c.stop());
            assert!(!c.batch_result(empty, String::new()));
            c.finish(empty);
            assert!(c.text.is_empty());
            assert_eq!(c.last_text, "complete text");
        }
    }
    #[test]
    fn fallback_before_output_and_during_drain() {
        let mut c = Controller::default();
        let old = c.start(false, false).unwrap();
        assert!(c.stop());
        c.realtime_failure(old);
        assert_eq!(c.phase, Phase::SubmittingBatch);
        assert!(!c.batch_result(old, "stale".into()));
        assert!(c.batch_result(c.ticket, "complete".into()));
        c.output_succeeded(c.ticket);
        c.finish(c.ticket);
        assert_eq!(c.last_text, "complete");
    }
    #[test]
    fn stale_and_repeated_stop() {
        let mut c = Controller::default();
        let old = c.start(false, false).unwrap();
        assert!(c.stop());
        assert!(!c.stop());
        assert!(c.delta(old, " hello"));
        c.finish(old);
        let now = c.start(true, true).unwrap();
        c.realtime_failure(old);
        c.finish(old);
        assert_eq!(c.phase, Phase::RecordingBatch);
        assert!(!c.delta(old, "oops"));
        assert_ne!(old, now);
    }
    #[test]
    fn fallback_preserves_full_text_without_reinsertion() {
        let mut c = Controller::default();
        let t = c.start(false, false).unwrap();
        assert!(c.delta(t, " first"));
        c.output_succeeded(t);
        c.realtime_failure(t);
        assert!(c.stop());
        assert!(!c.stop());
        let newer = c.ticket;
        assert!(!c.batch_result(t, "wrong".into()));
        assert!(!c.batch_result(newer, "complete result".into()));
        c.finish(newer);
        assert_eq!(c.last_text, "complete result");
        let empty = c.start(true, true).unwrap();
        assert!(c.stop());
        c.finish(empty);
        assert!(c.text.is_empty());
    }
    #[test]
    fn caps_text() {
        let mut c = Controller::default();
        let t = c.start(false, false).unwrap();
        assert!(!c.delta(t, &"x".repeat(MAX_TRANSCRIPT_BYTES + 1)));
        assert_eq!(c.phase, Phase::Failed);
    }
}
