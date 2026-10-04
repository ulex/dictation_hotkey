use std::collections::VecDeque;
/// Bounded worker -> UI queue, reserving capacity for final/error events.
pub struct Queue<T> {
    items: VecDeque<(T, usize)>,
    bytes: usize,
}
impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self {
            items: VecDeque::new(),
            bytes: 0,
        }
    }
}
impl<T> Queue<T> {
    pub fn push(&mut self, item: T, bytes: usize, terminal: bool) -> bool {
        let limit = if terminal { 768 * 1024 } else { 128 * 1024 };
        let count = if terminal { 64 } else { 60 };
        if self.items.len() >= count || self.bytes.checked_add(bytes).is_none_or(|n| n > limit) {
            return false;
        }
        self.bytes += bytes;
        self.items.push_back((item, bytes));
        true
    }
    pub fn pop(&mut self) -> Option<T> {
        self.items.pop_front().map(|(item, size)| {
            self.bytes -= size;
            item
        })
    }
    pub fn clear(&mut self) {
        self.items.clear();
        self.bytes = 0;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hostile_payloads_and_terminal_reservation() {
        let mut q = Queue::default();
        assert!(!q.push(0, usize::MAX, false));
        assert!(q.push(1, 128 * 1024, false));
        assert!(!q.push(2, 1, false));
        assert!(q.push(3, 256 * 1024, true));
        assert_eq!(q.pop(), Some(1));
        assert_eq!(q.pop(), Some(3));
        for i in 0..60 {
            assert!(q.push(i, 1, false));
        }
        assert!(!q.push(0, 1, false));
        assert!(q.push(99, 1, true));
        q.clear();
        assert!(q.pop().is_none());
    }
}
