//! ~1 MiB drop-oldest byte ring for Background Process output.

use std::collections::VecDeque;

pub struct RingBuffer {
    buf: VecDeque<u8>,
    capacity: usize,
}

impl RingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: VecDeque::with_capacity(capacity.min(64 * 1024)),
            capacity: capacity.max(1),
        }
    }

    pub fn write(&mut self, data: &[u8]) {
        for &b in data {
            if self.buf.len() >= self.capacity {
                self.buf.pop_front();
            }
            self.buf.push_back(b);
        }
    }

    pub fn as_lossy_string(&self) -> String {
        let bytes: Vec<u8> = self.buf.iter().copied().collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// Last `n` lines (newline-delimited), preserving incomplete trailing line.
    pub fn tail_lines(&self, n: usize) -> String {
        let text = self.as_lossy_string();
        if n == 0 || text.is_empty() {
            return String::new();
        }
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() <= n {
            // Preserve trailing newline if present.
            return text;
        }
        let start = lines.len() - n;
        let mut out = lines[start..].join("\n");
        if text.ends_with('\n') {
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_oldest_when_over_capacity() {
        let mut ring = RingBuffer::new(4);
        ring.write(b"abcdef");
        assert_eq!(ring.as_lossy_string(), "cdef");
    }

    #[test]
    fn tail_lines_returns_last_n() {
        let mut ring = RingBuffer::new(1024);
        ring.write(b"a\nb\nc\nd\n");
        assert_eq!(ring.tail_lines(2), "c\nd\n");
    }
}
