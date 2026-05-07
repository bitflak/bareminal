use core::clone::Clone;
use core::default::Default;
use core::option::Option;
use core::option::Option::None;
use core::option::Option::Some;
use core::result::Result;
use core::result::Result::Err;
use core::result::Result::Ok;

use heapless::Deque;

use crate::buffer::{self, Buffer};
use crate::process::RuntimeError;

/// Command line representation that tracks the cursor and history position
/// and manages the corresponding buffer.
#[derive(Default)]
pub struct CmdLine<const MAX_CMD_BUFFER: usize, const HISTORY_SIZE: usize> {
    cursor: usize,
    history: Deque<Buffer<MAX_CMD_BUFFER>, HISTORY_SIZE>,
    history_pos: usize,
    last_buffer: Option<Buffer<MAX_CMD_BUFFER>>,
    output: buffer::Buffer<MAX_CMD_BUFFER>,
    total: usize,
}

impl<const MAX_CMD_BUFFER: usize, const HISTORY_SIZE: usize> CmdLine<MAX_CMD_BUFFER, HISTORY_SIZE> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn prev_cmdline(&mut self) {
        if let Some(entry) = self.history.get_mut(self.history_pos).cloned() {
            if self.last_buffer.is_none() {
                self.last_buffer = Some(self.clone_buffer());
            }
            self.history_pos += 1;
            self.replace_output(entry);
        }
    }

    pub fn next_cmdline(&mut self) {
        self.history_pos = self.history_pos.saturating_sub(1);

        if self.history_pos == 0 {
            if let Some(ref last_buffer) = self.last_buffer {
                self.replace_output(last_buffer.clone());
                self.last_buffer = None;
            }
            return;
        }

        if let Some(entry) = self.history.get_mut(self.history_pos - 1).cloned() {
            if self.last_buffer.is_none() {
                self.last_buffer = Some(self.clone_buffer());
            }
            self.replace_output(entry);
        }
    }

    pub fn enqueue(&mut self) -> Result<(), RuntimeError> {
        if self.total == 0 {
            return Ok(());
        }

        if self.history.len() == HISTORY_SIZE {
            self.history.pop_back();
        }

        match self.history.push_front(self.clone_buffer()) {
            Err(_) => Err(RuntimeError::HistoryQueueError),
            _ => {
                self.reset();
                Ok(())
            }
        }
    }

    pub fn replace_output(&mut self, buffer: buffer::Buffer<MAX_CMD_BUFFER>) {
        let count = buffer.chars_count();
        self.output = buffer;
        self.cursor = count;
        self.total = count;
    }

    pub fn insert_char(&mut self, bytes: &[u8]) -> bool {
        if self.output.write_at_char_pos(bytes, self.cursor) {
            self.cursor += 1;
            self.total += 1;
            return true;
        }
        false
    }

    pub fn insert_chars(&mut self, str: &str) -> bool {
        let start_len = self.total;

        for (i, c) in str.char_indices() {
            let end = i + c.len_utf8();
            let bytes: &[u8] = &str.as_bytes()[i..end];
            self.insert_char(bytes);
        }

        start_len != self.total
    }

    pub fn move_left(&mut self) -> bool {
        if self.cursor > 0 {
            self.cursor -= 1;
            true
        } else {
            false
        }
    }

    pub fn move_right(&mut self) -> bool {
        if self.cursor < self.output.chars_count() {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    pub fn delete_char(&mut self) -> bool {
        if self.output.delete_char(self.cursor) {
            self.cursor -= 1;
            self.total -= 1;
            return true;
        }
        false
    }

    pub fn reset(&mut self) {
        self.last_buffer = None;
        self.history_pos = 0;
        self.cursor = 0;
        self.total = 0;
        self.output.reset();
    }

    pub fn reset_cursor(&mut self) {
        self.cursor = self.total;
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        self.output.as_bytes()
    }

    pub fn clone_buffer(&self) -> buffer::Buffer<MAX_CMD_BUFFER> {
        self.output.clone()
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    const MAX_BUF: usize = 64;
    const MAX_HIST: usize = 5;

    #[test]
    fn test_new() {
        let cmd = CmdLine::<MAX_BUF, MAX_HIST>::new();
        assert_eq!(cmd.as_bytes(), None);
    }

    #[test]
    fn test_insert_and_delete() {
        let mut cmd = CmdLine::<MAX_BUF, MAX_HIST>::new();
        assert!(cmd.insert_chars("hello"));
        assert_eq!(cmd.as_bytes().unwrap(), b"hello");
        assert_eq!(cmd.total, 5);

        for _ in 0..5 {
            assert!(cmd.move_left());
        }

        assert_eq!(cmd.cursor, 0);
        assert!(!cmd.move_left());

        assert!(cmd.move_right());
        assert_eq!(cmd.cursor, 1);

        assert!(cmd.delete_char());
        assert_eq!(cmd.as_bytes().unwrap(), b"ello");
        assert_eq!(cmd.total, 4);
    }

    #[test]
    fn test_multibyte() {
        let mut cmd = CmdLine::<MAX_BUF, MAX_HIST>::new();
        assert!(cmd.insert_chars("a🦀b"));
        assert_eq!(cmd.as_bytes().unwrap(), "a🦀b".as_bytes());
        assert_eq!(cmd.total, 3);

        assert!(cmd.move_left());
        assert!(cmd.delete_char());
        assert_eq!(cmd.as_bytes().unwrap(), b"ab");
        assert_eq!(cmd.total, 2);
    }

    #[test]
    fn test_enqueue_and_history() {
        let mut cmd = CmdLine::<MAX_BUF, MAX_HIST>::new();

        cmd.insert_chars("cmd1");
        cmd.enqueue().unwrap();
        assert_eq!(cmd.as_bytes(), None); // reset after enqueue
        assert_eq!(cmd.total, 0);

        cmd.insert_chars("cmd2");
        cmd.enqueue().unwrap();

        cmd.prev_cmdline();
        assert_eq!(cmd.as_bytes().unwrap(), b"cmd2");

        cmd.prev_cmdline();
        assert_eq!(cmd.as_bytes().unwrap(), b"cmd1");

        cmd.prev_cmdline();
        assert_eq!(cmd.as_bytes().unwrap(), b"cmd1");

        cmd.next_cmdline();
        assert_eq!(cmd.as_bytes().unwrap(), b"cmd2");

        cmd.next_cmdline();
        assert_eq!(cmd.as_bytes(), None);
    }

    #[test]
    fn test_enqueue_empty() {
        let mut cmd = CmdLine::<MAX_BUF, MAX_HIST>::new();
        cmd.enqueue().unwrap();
        assert_eq!(cmd.total, 0);
        cmd.prev_cmdline();
        assert_eq!(cmd.as_bytes(), None);
    }

    #[test]
    fn test_move_cursor() {
        let mut cmd = CmdLine::<MAX_BUF, MAX_HIST>::new();
        cmd.insert_chars("abc");
        assert_eq!(cmd.cursor, 3);

        assert!(cmd.move_left());
        assert_eq!(cmd.cursor, 2);
        assert!(cmd.move_left());
        assert_eq!(cmd.cursor, 1);
        assert!(cmd.move_left());
        assert_eq!(cmd.cursor, 0);
        assert!(!cmd.move_left());

        assert!(cmd.move_right());
        assert_eq!(cmd.cursor, 1);
        assert!(cmd.move_right());
        assert_eq!(cmd.cursor, 2);
        assert!(cmd.move_right());
        assert_eq!(cmd.cursor, 3);
        assert!(!cmd.move_right());
    }

    #[test]
    fn test_delete_middle() {
        let mut cmd = CmdLine::<MAX_BUF, MAX_HIST>::new();
        cmd.insert_chars("abcde");
        cmd.move_left();
        cmd.move_left();
        assert!(cmd.delete_char());
        assert_eq!(cmd.as_bytes().unwrap(), b"abde");
        assert_eq!(cmd.total, 4);
        assert_eq!(cmd.cursor, 2);
    }

    #[test]
    fn test_insert_multibyte_middle() {
        let mut cmd = CmdLine::<MAX_BUF, MAX_HIST>::new();
        cmd.insert_chars("a b");
        cmd.move_left();
        cmd.move_left();
        assert!(cmd.insert_chars("🦀"));
        assert_eq!(cmd.as_bytes().unwrap(), "a🦀 b".as_bytes());
        assert_eq!(cmd.total, 4);
        assert_eq!(cmd.cursor, 2);
    }

    #[test]
    fn test_reset() {
        let mut cmd = CmdLine::<MAX_BUF, MAX_HIST>::new();
        cmd.insert_chars("test");
        cmd.reset();
        assert_eq!(cmd.total, 0);
        assert_eq!(cmd.cursor, 0);
        assert_eq!(cmd.as_bytes(), None);
    }

    #[test]
    fn test_reset_cursor() {
        let mut cmd = CmdLine::<MAX_BUF, MAX_HIST>::new();
        cmd.insert_chars("test");
        cmd.move_left();
        assert_eq!(cmd.cursor, 3);
        cmd.reset_cursor();
        assert_eq!(cmd.cursor, 4);
    }

    #[test]
    fn test_complex_history() {
        let mut cmd = CmdLine::<MAX_BUF, MAX_HIST>::new();

        cmd.insert_chars("cmd1");
        cmd.enqueue().unwrap();

        cmd.insert_chars("cmd2");
        cmd.enqueue().unwrap();

        cmd.insert_chars("cmd3");
        cmd.enqueue().unwrap();

        cmd.prev_cmdline();
        assert_eq!(cmd.as_bytes().unwrap(), b"cmd3");

        cmd.prev_cmdline();
        assert_eq!(cmd.as_bytes().unwrap(), b"cmd2");

        cmd.prev_cmdline();
        assert_eq!(cmd.as_bytes().unwrap(), b"cmd1");

        cmd.prev_cmdline();
        assert_eq!(cmd.as_bytes().unwrap(), b"cmd1");

        cmd.next_cmdline();
        assert_eq!(cmd.as_bytes().unwrap(), b"cmd2");

        cmd.next_cmdline();
        assert_eq!(cmd.as_bytes().unwrap(), b"cmd3");

        cmd.next_cmdline();
        assert_eq!(cmd.as_bytes(), None);
    }
}
