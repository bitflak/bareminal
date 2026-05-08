use core::convert::AsRef;
use core::iter::IntoIterator;
use core::iter::Iterator;
use core::ops::FnOnce;
use core::option::Option::Some;
use core::result::Result;
use core::result::Result::Err;
use core::result::Result::Ok;
use core::str::FromStr;
use std::io::Write;

use crate::{
    buffer::Buffer,
    bytes::*,
    cmdline,
    input::{Control, InputParser, InputType},
    process::{CommandsParser, ProcessError, RuntimeError},
};

pub struct CommandWriter<W: Write> {
    writer: W,
}

impl<W: Write> CommandWriter<W> {
    /// Attempts to write an entire buffer
    #[inline(always)]
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), RuntimeError> {
        self.writer
            .write_all(bytes)
            .map_err(|_| RuntimeError::WriteFailed)
    }

    /// Attempts to write an entire buffer and flush the writer
    #[inline(always)]
    pub fn flush_write(&mut self, bytes: &[u8]) -> Result<(), RuntimeError> {
        self.writer
            .write_all(bytes)
            .map_err(|_| RuntimeError::WriteFailed)?;
        self.flush()
    }

    /// Writes a single new line with carriage return new line feed
    pub fn write_line(&mut self, bytes: &[u8]) -> Result<(), RuntimeError> {
        self.write(bytes)?;
        self.write(CRLF)
    }

    /// Writes each element as a new line
    pub fn write_lines<I, S>(&mut self, lines: I) -> Result<(), RuntimeError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for line in lines {
            self.write_line(line.as_ref().as_bytes())?;
        }

        Ok(())
    }

    /// Flushes the inner writer
    #[inline(always)]
    pub fn flush(&mut self) -> Result<(), RuntimeError> {
        self.writer.flush().map_err(|_| RuntimeError::WriteFailed)
    }

    pub fn write_error(&mut self, error: &[u8]) -> Result<(), RuntimeError> {
        self.write(CRCL)?;
        self.write(BOLD_RED)?;
        self.write(error)?;
        self.write(RESET_COLOR)?;
        self.write(CRLF)
    }

    pub fn write_buffer<const BUFFER_SIZE: usize>(
        &mut self,
        buffer: &mut Buffer<BUFFER_SIZE>,
    ) -> Result<(), RuntimeError> {
        self.write(CRCL)?;
        self.write(START_SYNC)?;
        if let Some(bytes) = buffer.as_bytes() {
            self.write(bytes)?;
            self.write(CRLF)?;
        }
        self.write(END_SYNC)
    }

    /// Format a table and append it to the buffer.
    ///
    /// On `BufferTooSmall`, the buffer may contain a partial table
    ///
    /// Layout:
    /// ```text
    /// | header1 | header2 |
    /// |---------|---------|
    /// | cell    | cell    |
    /// ```
    pub fn write_table<const BUFFER_SIZE: usize>(
        &mut self,
        headers: &[&str],
        rows: &[&[&str]],
    ) -> Result<(), RuntimeError> {
        let mut buffer = Buffer::<BUFFER_SIZE>::new();
        buffer.write_table(headers, rows)?;
        self.write_buffer(&mut buffer)
    }

    /// Format a key/value list and append it to the buffer.
    ///
    /// Each entry must have exactly two cells: a name and a value.
    /// Names are padded so all values align to the same column.
    ///
    /// If `enumerate` is true, each row is prefixed with its 1-based index
    /// (e.g. ` 1. name:   value`). Index numbers are right-aligned so the
    /// names stay vertically aligned regardless of how many entries there are.
    pub fn write_list<const BUFFER_SIZE: usize>(
        &mut self,
        entries: &[&[&str]],
        enumerate: bool,
    ) -> Result<(), RuntimeError> {
        let mut buffer = Buffer::<BUFFER_SIZE>::new();
        buffer.write_list(entries, enumerate)?;
        self.write_buffer(&mut buffer)
    }

    /// Serialize a value as pretty-printed JSON and append it to the buffer.
    /// Uses 2-space indentation.
    pub fn write_json<const BUFFER_SIZE: usize>(
        &mut self,
        value: &impl serde::Serialize,
        pretty: bool,
    ) -> Result<(), RuntimeError> {
        let mut buffer = Buffer::<BUFFER_SIZE>::new();
        buffer.write_json(value, pretty)?;
        self.write_buffer(&mut buffer)
    }
}

pub struct Bareminal<C, W: Write, const MAX_CMD_BUFFER: usize = 256, const HISTORY_SIZE: usize = 3>
{
    processor: InputParser,
    writer: CommandWriter<W>,
    cmdline: cmdline::CmdLine<MAX_CMD_BUFFER, HISTORY_SIZE>,
    _phantom: core::marker::PhantomData<C>,
}

impl<C, W, const MAX_CMD_BUFFER: usize, const HISTORY_SIZE: usize>
    Bareminal<C, W, MAX_CMD_BUFFER, HISTORY_SIZE>
where
    C: CommandsParser,
    W: Write,
{
    pub fn new(writer: W) -> Result<Self, RuntimeError> {
        let mut cli = Self {
            processor: InputParser::new(),
            cmdline: cmdline::CmdLine::<MAX_CMD_BUFFER, HISTORY_SIZE>::new(),
            writer: CommandWriter { writer },
            _phantom: core::marker::PhantomData,
        };

        cli.init_prompt()?;

        Ok(cli)
    }

    pub fn init_prompt(&mut self) -> Result<(), RuntimeError> {
        self.writer
            .write(PROMPT)
            .map_err(|_| RuntimeError::WriteFailed)?;
        self.writer.flush().map_err(|_| RuntimeError::FlushFailed)
    }

    /// Writes the provided bytes in the current line and
    /// redraws the prompt on a new line
    fn redraw_write(&mut self, bytes: &[u8]) -> Result<(), RuntimeError> {
        self.writer.write(CRCL)?;
        self.writer.write_line(bytes)?;
        self.redraw_prompt()
    }

    /// Redraws the prompt in the current line
    fn redraw_prompt(&mut self) -> Result<(), RuntimeError> {
        self.write(CRCLPROMPT)?;
        if let Some(cmdline_bytes) = self.cmdline.as_bytes() {
            self.writer
                .write(cmdline_bytes)
                .map_err(|_| RuntimeError::WriteFailed)?;
            self.cmdline.reset_cursor();
        }
        Ok(())
    }

    pub fn parse_command(
        command: &str,
        handle: impl for<'a> FnOnce(Result<C::Match<'a>, ProcessError<'a>>),
    ) {
        if let Ok(mut buffer) = Buffer::<MAX_CMD_BUFFER>::from_str(command)
            && let Some(tokens) = buffer.as_tokens()
        {
            handle(C::parse(&mut tokens.iter()));
        }
    }

    pub fn add_byte(
        &mut self,
        byte: u8,
        handle: impl for<'a> Fn(C::Match<'a>, &mut CommandWriter<W>),
    ) -> Result<(), RuntimeError> {
        if let Some(result) = self.processor.push_byte(byte) {
            match result {
                InputType::Ctrl(control) => {
                    match control {
                        Control::Backspace => {
                            if self.cmdline.delete_char() {
                                self.writer.write(DELETE_CHAR_LEFT)?;
                            }
                        }
                        Control::Enter => {
                            let mut buffer_copy = self.cmdline.clone_buffer();
                            if let Some(tokens) = buffer_copy.as_tokens() {
                                if let Some(cmd) = tokens.iter().peek()
                                    && cmd == "help"
                                {
                                    let mut tokens = tokens.iter();
                                    let _ = tokens.next();
                                    self.cmdline.enqueue()?;
                                    self.cmdline.reset();
                                    self.writer.write(CRLF)?;
                                    self.writer.write(START_SYNC)?;
                                    if let Some(cmd_name) = tokens.next() {
                                        let help = C::help_for(cmd_name);
                                        if help.is_empty() {
                                            self.writer.write_line("Unknown command".as_bytes())?;
                                        }
                                        self.writer.write_lines(help)?;
                                    } else {
                                        let help = C::help_lines();
                                        self.writer.write_lines(help)?;
                                    }
                                    self.writer.write(END_SYNC)?;
                                    self.redraw_prompt()?;
                                } else {
                                    let mut tokens_iter = tokens.iter();
                                    let mut count: u8 = 0;
                                    while tokens_iter.peek().is_some() {
                                        match C::parse(&mut tokens_iter) {
                                            Ok(command) => {
                                                if count < 1 {
                                                    self.writer.write(CRLFPROMPT)?;
                                                    count += 1;
                                                }
                                                self.cmdline.enqueue()?;
                                                self.cmdline.reset();
                                                self.writer.write(CRCL)?;
                                                handle(command, &mut self.writer);
                                            }
                                            Err(err) => {
                                                self.writer.write(CRCL)?;
                                                self.writer.write(BOLD_RED)?;
                                                if let Ok(err_str) =
                                                    err.to_string::<MAX_CMD_BUFFER>()
                                                {
                                                    self.writer.write_line(err_str.as_bytes())?;
                                                } else {
                                                    self.writer.write_line(
                                                        "Error buffer too long".as_bytes(),
                                                    )?;
                                                }
                                                self.writer.write(RESET_COLOR)?;
                                                break;
                                            }
                                        }
                                    }
                                    self.redraw_prompt()?;
                                }
                            } else {
                                self.redraw_write("Run help for more information".as_bytes())?;
                            }
                        }
                        Control::Left => {
                            if self.cmdline.move_left() {
                                self.writer.write(CURSOR_LEFT)?;
                            }
                        }
                        Control::Right => {
                            if self.cmdline.move_right() {
                                self.writer.write(CURSOR_RIGHT)?;
                            }
                        }
                        Control::Down => {
                            self.cmdline.next_cmdline();
                            self.redraw_prompt()?;
                        }
                        Control::Formfeed => {
                            self.cmdline.reset();
                            self.writer.write(CRCLPROMPT)?;
                        }
                        Control::Tab => {
                            let mut buffer_copy = self.cmdline.clone_buffer();
                            if let Some(tokens) = buffer_copy.as_tokens()
                                && let Some(last_token) = tokens.iter().peek_last()
                                && let Some(completion) = C::autocomplete(last_token)
                                && let Some(suffix) = completion.strip_prefix(last_token)
                                && self.cmdline.insert_chars(suffix)
                            {
                                self.redraw_prompt()?;
                            }
                        }
                        Control::Up => {
                            self.cmdline.prev_cmdline();
                            self.redraw_prompt()?;
                        }
                        Control::Exit => {}
                    };
                }
                InputType::Char(char) => {
                    let bytes = char.as_bytes();
                    if self.cmdline.insert_char(bytes) {
                        self.writer.write(INSERT_CHAR)?;
                        self.writer.write(bytes)?;
                    }
                }
            }
        }
        self.writer.flush()?;
        Ok(())
    }

    pub fn write(&mut self, buf: &[u8]) -> Result<(), RuntimeError> {
        self.writer.flush_write(buf)
    }

    pub fn writer(&mut self) -> &mut W {
        &mut self.writer.writer
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::process::{CommandsParser, HelpIter, ProcessError};
    use crate::tokens::TokensIter;
    use std::cell::RefCell;
    use std::io::{self, Write};
    use std::rc::Rc;

    #[derive(Clone, Default)]
    struct MockWriter {
        captured: Rc<RefCell<Vec<u8>>>,
    }

    impl MockWriter {
        fn new() -> Self {
            Self::default()
        }

        fn as_str(&self) -> String {
            String::from_utf8_lossy(&self.captured.borrow()).to_string()
        }

        fn clear(&self) {
            self.captured.borrow_mut().clear();
        }
    }

    impl Write for MockWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.captured.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[derive(Debug, PartialEq)]
    enum TestCommand<'a> {
        Hello,
        Echo(&'a str),
    }

    impl CommandsParser for TestCommand<'_> {
        type Match<'m> = TestCommand<'m>;

        fn parse<'p>(tokens: &mut TokensIter<'p>) -> Result<Self::Match<'p>, ProcessError<'p>> {
            while tokens.peek() == Some("--") {
                tokens.next();
            }
            let name = tokens.next().ok_or(ProcessError::Empty)?;
            match name {
                "hello" => Ok(TestCommand::Hello),
                "echo" => {
                    let value = tokens.next().ok_or(ProcessError::Empty)?;
                    Ok(TestCommand::Echo(value))
                }
                _ => Err(ProcessError::Unknown),
            }
        }

        fn help() -> &'static [&'static str] {
            &["Test commands:", "  hello", "  echo <text>"]
        }

        fn help_for(name: &str) -> &'static [&'static str] {
            match name {
                "hello" => &["== hello ==", "say hello", "hello"],
                "echo" => &["== echo ==", "echo a value", "echo <text>"],
                _ => &[],
            }
        }

        fn autocomplete(name: &str) -> Option<&'static str> {
            const NAMES: &[&str] = &["help", "hello", "echo"];
            let mut found = None;
            let mut return_next = false;
            for &c in NAMES {
                if c.starts_with(name) {
                    if return_next {
                        return Some(c);
                    }
                    if found.is_none() {
                        found = Some(c);
                    }
                    if c == name {
                        return_next = true;
                    }
                }
            }
            found
        }

        fn help_lines() -> HelpIter {
            HelpIter::single(Self::help())
        }
    }

    /// Drives a sequence of bytes through `add_byte` with a no-op handler
    /// that records dispatched commands into a Vec.
    fn run_input(
        cli: &mut Bareminal<TestCommand<'static>, MockWriter, 256, 4>,
        input: &[u8],
    ) -> Vec<String> {
        let dispatched = Rc::new(RefCell::new(Vec::<String>::new()));
        let dispatched_clone = dispatched.clone();
        for &byte in input {
            cli.add_byte(byte, |cmd, _writer| {
                dispatched_clone.borrow_mut().push(format!("{:?}", cmd));
            })
            .unwrap();
        }
        dispatched.borrow().clone()
    }

    #[test]
    fn new_writes_initial_prompt() {
        let mw = MockWriter::new();
        let _cli: Bareminal<TestCommand, _, 256, 4> = Bareminal::new(mw.clone()).unwrap();
        let out = mw.as_str();
        assert!(!out.is_empty(), "expected prompt to be written");
    }

    #[test]
    fn typing_chars_does_not_dispatch() {
        let mw = MockWriter::new();
        let mut cli: Bareminal<TestCommand, _, 256, 4> = Bareminal::new(mw.clone()).unwrap();
        let dispatched = run_input(&mut cli, b"hello");
        assert!(
            dispatched.is_empty(),
            "no command should be dispatched while typing: {:?}",
            dispatched
        );
    }

    #[test]
    fn enter_dispatches_parsed_command() {
        let mw = MockWriter::new();
        let mut cli: Bareminal<TestCommand, _, 256, 4> = Bareminal::new(mw.clone()).unwrap();
        let dispatched = run_input(&mut cli, b"hello\r");
        assert_eq!(dispatched.len(), 1);
        assert_eq!(dispatched[0], "Hello");
    }

    #[test]
    fn echo_command_with_argument() {
        let mw = MockWriter::new();
        let mut cli: Bareminal<TestCommand, _, 256, 4> = Bareminal::new(mw.clone()).unwrap();
        let dispatched = run_input(&mut cli, b"echo world\r");
        assert_eq!(dispatched.len(), 1);
        assert!(
            dispatched[0].contains("Echo") && dispatched[0].contains("world"),
            "unexpected dispatch: {:?}",
            dispatched[0]
        );
    }

    #[test]
    fn multiple_commands_chain_with_separator() {
        let mw = MockWriter::new();
        let mut cli: Bareminal<TestCommand, _, 256, 4> = Bareminal::new(mw.clone()).unwrap();
        let dispatched = run_input(&mut cli, b"hello -- echo world\r");
        assert_eq!(
            dispatched.len(),
            2,
            "expected two commands, got: {:?}",
            dispatched
        );
        assert_eq!(dispatched[0], "Hello");
        assert!(dispatched[1].contains("Echo"));
    }

    #[test]
    fn unknown_command_writes_error_and_aborts_chain() {
        let mw = MockWriter::new();
        let mut cli: Bareminal<TestCommand, _, 256, 4> = Bareminal::new(mw.clone()).unwrap();
        // First command is unknown; second should not be processed.
        let dispatched = run_input(&mut cli, b"bogus -- hello\r");
        assert_eq!(
            dispatched.len(),
            0,
            "no command should be dispatched on parse error"
        );
        let out = mw.as_str();
        assert!(
            out.to_lowercase().contains("unknown") || out.contains("\x1b["),
            "expected error indication in output: {:?}",
            out
        );
    }

    #[test]
    fn help_command_writes_top_level_help() {
        let mw = MockWriter::new();
        let mut cli: Bareminal<TestCommand, _, 256, 4> = Bareminal::new(mw.clone()).unwrap();
        mw.clear();
        let dispatched = run_input(&mut cli, b"help\r");
        assert!(dispatched.is_empty(), "help shouldn't dispatch a command");
        let out = mw.as_str();
        assert!(
            out.contains("Test commands"),
            "expected help text in output: {:?}",
            out
        );
    }

    #[test]
    fn help_for_specific_command() {
        let mw = MockWriter::new();
        let mut cli: Bareminal<TestCommand, _, 256, 4> = Bareminal::new(mw.clone()).unwrap();
        mw.clear();
        let dispatched = run_input(&mut cli, b"help hello\r");
        assert!(dispatched.is_empty());
        let out = mw.as_str();
        assert!(
            out.contains("hello") && out.contains("say hello"),
            "expected per-command help: {:?}",
            out
        );
    }

    #[test]
    fn help_for_unknown_command_writes_unknown_message() {
        let mw = MockWriter::new();
        let mut cli: Bareminal<TestCommand, _, 256, 4> = Bareminal::new(mw.clone()).unwrap();
        mw.clear();
        run_input(&mut cli, b"help bogus\r");
        let out = mw.as_str();
        assert!(
            out.contains("Unknown command"),
            "expected 'Unknown command': {:?}",
            out
        );
    }

    #[test]
    fn backspace_removes_char_before_dispatch() {
        let mw = MockWriter::new();
        let mut cli: Bareminal<TestCommand, _, 256, 4> = Bareminal::new(mw.clone()).unwrap();
        // Type "hellox", backspace once → "hello", press Enter.
        let mut input = b"hellox".to_vec();
        input.push(0x7f); // DEL / backspace
        input.push(b'\r');
        let dispatched = run_input(&mut cli, &input);
        assert_eq!(dispatched.len(), 1);
        assert_eq!(dispatched[0], "Hello");
    }

    #[test]
    fn empty_enter_writes_help_hint() {
        let mw = MockWriter::new();
        let mut cli: Bareminal<TestCommand, _, 256, 4> = Bareminal::new(mw.clone()).unwrap();
        mw.clear();
        let dispatched = run_input(&mut cli, b"\r");
        assert!(dispatched.is_empty());
        let out = mw.as_str();
        assert!(
            out.contains("help") || out.contains("Run"),
            "expected help-hint text: {:?}",
            out
        );
    }

    #[test]
    fn tab_completes_unique_prefix() {
        let mw = MockWriter::new();
        let mut cli: Bareminal<TestCommand, _, 256, 4> = Bareminal::new(mw.clone()).unwrap();
        // "ec" → "echo" (unique prefix), then space + arg + Enter
        let mut input = b"ec".to_vec();
        input.push(b'\t');
        input.extend_from_slice(b" hi\r");
        let dispatched = run_input(&mut cli, &input);
        assert_eq!(dispatched.len(), 1, "got: {:?}", dispatched);
        assert!(
            dispatched[0].contains("Echo"),
            "expected Echo: {:?}",
            dispatched[0]
        );
    }

    #[test]
    fn parse_command_static_helper() {
        let mut got: Option<String> = None;
        Bareminal::<TestCommand, MockWriter, 256, 4>::parse_command("hello", |result| {
            got = Some(format!("{:?}", result));
        });
        assert!(got.is_some());
        let s = got.unwrap();
        assert!(s.contains("Hello"), "unexpected parse: {:?}", s);
    }

    #[test]
    fn parse_command_with_invalid_input() {
        let mut got: Option<String> = None;
        Bareminal::<TestCommand, MockWriter, 256, 4>::parse_command("bogus", |result| {
            got = Some(format!("{:?}", result));
        });
        let s = got.unwrap();
        assert!(s.contains("Unknown") || s.contains("Err"), "got: {:?}", s);
    }

    #[test]
    fn long_input_within_buffer_works() {
        let mw = MockWriter::new();
        let mut cli: Bareminal<TestCommand, _, 256, 4> = Bareminal::new(mw.clone()).unwrap();
        let mut input = b"echo ".to_vec();
        input.extend(std::iter::repeat_n(b'x', 50));
        input.push(b'\r');
        let dispatched = run_input(&mut cli, &input);
        assert_eq!(dispatched.len(), 1);
    }

    #[test]
    fn arrow_up_recalls_history_after_enqueue() {
        let mw = MockWriter::new();
        let mut cli: Bareminal<TestCommand, _, 256, 4> = Bareminal::new(mw.clone()).unwrap();

        // Rype and execute a command so it gets enqueued.
        run_input(&mut cli, b"hello\r");

        // Press Up arrow
        run_input(&mut cli, b"\x1b[A");

        // Press Enter — should re-execute the recalled "hello".
        let dispatched = run_input(&mut cli, b"\r");
        assert_eq!(
            dispatched.len(),
            1,
            "expected history recall to execute previous command"
        );
        assert_eq!(dispatched[0], "Hello");
    }
}
