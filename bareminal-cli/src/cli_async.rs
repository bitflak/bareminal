use core::convert::AsRef;
use core::future::ready;
use core::iter::IntoIterator;
use core::iter::Iterator;
use core::marker::Sized;
use core::marker::Unpin;
use core::ops::AsyncFnMut;
use core::ops::FnOnce;
use core::option::Option;
use core::option::Option::None;
use core::option::Option::Some;
use core::result::Result;
use core::result::Result::Err;
use core::result::Result::Ok;
use core::str::FromStr;

#[cfg(feature = "async-std")]
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[cfg(all(feature = "async-no-std", not(feature = "async-std")))]
use embassy_futures::select::{Either, select};

#[cfg(all(feature = "async-no-std", not(feature = "async-std")))]
use embedded_io_async::{Read as AsyncRead, Write as AsyncWrite};

use crate::{
    buffer::Buffer,
    bytes::*,
    cmdline,
    input::{Control, InputParser, InputType},
    process::{CommandsParser, ProcessError, RuntimeError},
};

pub struct CommandWriter<W: AsyncWrite + Unpin> {
    pub writer: W,
    pub is_dirty: bool,
}

impl<W: AsyncWrite + Unpin> CommandWriter<W> {
    fn new(writer: W) -> Self {
        Self {
            writer,
            is_dirty: false,
        }
    }

    /// Attempts to write an entire buffer
    #[inline(always)]
    pub async fn write(&mut self, bytes: &[u8]) -> Result<(), RuntimeError> {
        self.is_dirty = true;
        self.writer
            .write_all(bytes)
            .await
            .map_err(|_| RuntimeError::WriteFailed)
    }

    /// Attempts to write an entire buffer and flush the writer
    #[inline(always)]
    pub async fn flush_write(&mut self, bytes: &[u8]) -> Result<(), RuntimeError> {
        self.is_dirty = true;
        self.writer
            .write_all(bytes)
            .await
            .map_err(|_| RuntimeError::WriteFailed)?;
        self.flush().await
    }

    /// Writes a single new line with carriage return new line feed
    pub async fn write_line(&mut self, bytes: &[u8]) -> Result<(), RuntimeError> {
        self.is_dirty = true;
        self.write(bytes).await?;
        self.write(CRLF).await
    }

    /// Writes each element as a new line
    pub async fn write_lines<I, S>(&mut self, lines: I) -> Result<(), RuntimeError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.is_dirty = true;
        for line in lines {
            self.write_line(line.as_ref().as_bytes()).await?;
        }

        Ok(())
    }

    /// Flushes the inner writer
    #[inline(always)]
    pub async fn flush(&mut self) -> Result<(), RuntimeError> {
        // Ensure there is at least one byte to flush
        // otherwise embedded_io_async flush can wait indefinitely
        if !self.is_dirty {
            self.writer
                .write_all(b"\0")
                .await
                .map_err(|_| RuntimeError::FlushFailed)?;
        } else {
            self.is_dirty = false;
        }

        self.writer
            .flush()
            .await
            .map_err(|_| RuntimeError::FlushFailed)
    }

    pub async fn write_error(&mut self, error: &[u8]) -> Result<(), RuntimeError> {
        self.is_dirty = true;
        self.write(CRCL).await?;
        self.write(BOLD_RED).await?;
        self.write(error).await?;
        self.write(RESET_COLOR).await?;
        self.write(CRLF).await
    }

    pub async fn write_buffer<const BUFFER_SIZE: usize>(
        &mut self,
        buffer: &mut Buffer<BUFFER_SIZE>,
    ) -> Result<(), RuntimeError> {
        self.is_dirty = true;
        self.write(CRCL).await?;
        self.write(START_SYNC).await?;
        if let Some(bytes) = buffer.as_bytes() {
            self.write(bytes).await?;
            self.write(CRLF).await?;
        }
        self.write(END_SYNC).await
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
    pub async fn write_table<const BUFFER_SIZE: usize>(
        &mut self,
        headers: &[&str],
        rows: &[&[&str]],
    ) -> Result<(), RuntimeError> {
        self.is_dirty = true;
        let mut buffer = Buffer::<BUFFER_SIZE>::new();
        buffer.write_table(headers, rows)?;
        self.write_buffer(&mut buffer).await
    }

    /// Format a key/value list and append it to the buffer.
    ///
    /// Each entry must have exactly two cells: a name and a value.
    /// Names are padded so all values align to the same column.
    ///
    /// If `enumerate` is true, each row is prefixed with its 1-based index
    /// (e.g. ` 1. name:   value`). Index numbers are right-aligned so the
    /// names stay vertically aligned regardless of how many entries there are.
    pub async fn write_list<const BUFFER_SIZE: usize>(
        &mut self,
        entries: &[&[&str]],
        enumerate: bool,
    ) -> Result<(), RuntimeError> {
        self.is_dirty = true;
        let mut buffer = Buffer::<BUFFER_SIZE>::new();
        buffer.write_list(entries, enumerate)?;
        self.write_buffer(&mut buffer).await
    }

    /// Serialize a value as pretty-printed JSON and append it to the buffer.
    /// Uses 2-space indentation.
    pub async fn write_json<const BUFFER_SIZE: usize>(
        &mut self,
        value: &impl serde::Serialize,
        pretty: bool,
    ) -> Result<(), RuntimeError> {
        self.is_dirty = true;
        let mut buffer = Buffer::<BUFFER_SIZE>::new();
        buffer.write_json(value, pretty)?;
        self.write_buffer(&mut buffer).await
    }

    /// Hides the command line and clears the screen, then switches into a continuous rendering
    /// mode. The provided callback receives a fixed size mutable buffer, the contents of the buffer
    /// can be written and returned back. Also continuously watching for a keypress "q" that stops
    /// the loop and returns back into the command line mode.
    ///
    /// Note: The function is the way it is not because of rust, but because AsyncFn + HRTB breaks
    /// rust analyzer and the requirement to be able to run in no_std environment without
    /// allocations leaves no other options
    pub async fn write_loop<const BUFFER_SIZE: usize>(
        &mut self,
        reader: &mut (impl AsyncRead + Unpin + ?Sized),
        mut handle: impl AsyncFnMut(Buffer<BUFFER_SIZE>) -> Buffer<BUFFER_SIZE>,
    ) -> Result<(), RuntimeError> {
        let mut buf = [0u8; 1];
        let mut buffer = Buffer::<BUFFER_SIZE>::new();

        self.write(HIDE_CURSOR).await?;
        let msg = b"-- press q to quit --\n\r";

        loop {
            // Hand the buffer to the closure, get it back filled.
            buffer = handle(buffer).await;

            self.write(START_SYNC_AND_ERASE_SCREEN).await?;
            self.write(msg).await?;
            if let Some(bytes) = buffer.as_bytes() {
                self.write(bytes).await?;
            }
            self.write(END_SYNC).await?;
            self.flush().await?;

            buffer.reset();

            #[cfg(feature = "async-std")]
            tokio::select! {
                biased;
                result = reader.read(&mut buf) => {
                    match result {
                        Ok(0) | Err(_) => break,
                        Ok(_) if buf[0] == b'q' => break,
                        Ok(_) => {}
                    }
                }
                _ = ready(()) => {}
            }

            #[cfg(all(feature = "async-no-std", not(feature = "async-std")))]
            match select(reader.read(&mut buf), ready(())).await {
                Either::First(result) => match result {
                    Ok(0) | Err(_) => break,
                    Ok(_) if buf[0] == b'q' => break,
                    Ok(_) => {}
                },
                Either::Second(_) => {}
            }
        }

        let _ = self.writer.write_all(ERASE_SCREEN).await;
        let _ = self.writer.write_all(SHOW_CURSOR).await;
        Ok(())
    }
}

pub struct Bareminal<C, W, const MCB: usize, const H: usize>
where
    C: CommandsParser,
    W: AsyncWrite + Unpin,
{
    processor: InputParser,
    cmdline: cmdline::CmdLine<MCB, H>,
    writer: CommandWriter<W>,
    pending_buffer: Option<Buffer<MCB>>,
    pending_offset: usize,
    pending_active: bool,
    _phantom: core::marker::PhantomData<C>,
}

impl<C, W, const MCB: usize, const H: usize> Bareminal<C, W, MCB, H>
where
    C: CommandsParser,
    W: AsyncWrite + Unpin,
{
    pub async fn new(writer: W) -> Result<Self, RuntimeError> {
        let mut cli = Self {
            processor: InputParser::new(),
            cmdline: cmdline::CmdLine::new(),
            writer: CommandWriter::new(writer),
            pending_buffer: None,
            pending_offset: 0,
            pending_active: false,
            _phantom: core::marker::PhantomData,
        };

        cli.init_prompt().await?;

        Ok(cli)
    }

    async fn init_prompt(&mut self) -> Result<(), RuntimeError> {
        self.writer
            .write(PROMPT)
            .await
            .map_err(|_| RuntimeError::WriteFailed)?;
        self.writer
            .flush()
            .await
            .map_err(|_| RuntimeError::FlushFailed)
    }

    /// Writes the provided bytes in the current line and
    /// redraws the prompt on a new line
    pub async fn redraw_write(&mut self, bytes: &[u8]) -> Result<(), RuntimeError> {
        self.writer.write(CRCL).await?;
        self.writer.write_line(bytes).await?;
        self.redraw_prompt().await
    }

    /// Redraws the prompt in the current line
    pub async fn redraw_prompt(&mut self) -> Result<(), RuntimeError> {
        self.writer.write(CRCLPROMPT).await?;
        if let Some(cmdline_bytes) = self.cmdline.as_bytes() {
            self.writer
                .write(cmdline_bytes)
                .await
                .map_err(|_| RuntimeError::WriteFailed)?;
            self.cmdline.reset_cursor();
        }
        Ok(())
    }

    pub fn parse_command(
        command: &str,
        handle: impl for<'a> FnOnce(Result<C::Match<'a>, ProcessError<'a>>),
    ) {
        if let Ok(mut buffer) = Buffer::<MCB>::from_str(command)
            && let Some(tokens) = buffer.as_tokens()
        {
            handle(C::parse(&mut tokens.iter()));
        }
    }

    pub async fn add_byte(&mut self, byte: u8) -> Result<bool, RuntimeError> {
        if let Some(result) = self.processor.push_byte(byte) {
            match result {
                InputType::Ctrl(control) => {
                    match control {
                        Control::Backspace => {
                            if self.cmdline.delete_char() {
                                self.writer.flush_write(DELETE_CHAR_LEFT).await?;
                            }
                        }
                        Control::Enter => {
                            self.pending_buffer = Some(self.cmdline.clone_buffer());
                            self.pending_offset = 0;
                            self.pending_active = true;
                            return Ok(true);
                        }
                        Control::Left => {
                            if self.cmdline.move_left() {
                                self.writer.flush_write(CURSOR_LEFT).await?;
                            }
                        }
                        Control::Right => {
                            if self.cmdline.move_right() {
                                self.writer.flush_write(CURSOR_RIGHT).await?;
                            }
                        }
                        Control::Down => {
                            self.cmdline.next_cmdline();
                            self.redraw_prompt().await?;
                            self.writer.flush().await?;
                        }
                        Control::Formfeed => {
                            self.cmdline.reset();
                            self.writer.flush_write(CRCLPROMPT).await?;
                        }
                        Control::Tab => {
                            let mut buffer_copy = self.cmdline.clone_buffer();
                            if let Some(tokens) = buffer_copy.as_tokens()
                                && let Some(last_token) = tokens.iter().peek_last()
                                && let Some(completion) = C::autocomplete(last_token)
                                && let Some(suffix) = completion.strip_prefix(last_token)
                                && self.cmdline.insert_chars(suffix)
                            {
                                self.redraw_prompt().await?;
                                self.writer.flush().await?;
                            }
                        }
                        Control::Up => {
                            self.cmdline.prev_cmdline();
                            self.redraw_prompt().await?;
                            self.writer.flush().await?;
                        }
                        Control::Exit => {}
                    };
                }
                InputType::Char(char) => {
                    let bytes = char.as_bytes();
                    if self.cmdline.insert_char(bytes) {
                        self.writer.write(INSERT_CHAR).await?;
                        self.writer.write(bytes).await?;
                    }
                    self.redraw_prompt().await?;
                    self.writer.flush().await?;
                }
            }
        }
        Ok(false)
    }

    pub async fn next_command<'a>(
        &'a mut self,
    ) -> Result<Option<(C::Match<'a>, &'a mut CommandWriter<W>)>, RuntimeError> {
        let buf = match self.pending_buffer.as_mut() {
            Some(b) => b,
            None => return Ok(None),
        };

        let tokens = match buf.as_tokens() {
            Some(t) => t,
            None => {
                if self.pending_offset == 0 {
                    self.writer.write(CRCL).await?;
                    self.writer
                        .write_line("Run help for more information".as_bytes())
                        .await?;
                    self.writer.write(CRCLPROMPT).await?;
                    if let Some(cmdline_bytes) = self.cmdline.as_bytes() {
                        self.writer
                            .write(cmdline_bytes)
                            .await
                            .map_err(|_| RuntimeError::WriteFailed)?;
                        self.cmdline.reset_cursor();
                    }
                }
                return Ok(None);
            }
        };

        let mut iter = tokens.iter_from(self.pending_offset);

        if let Some(cmd) = iter.peek()
            && cmd == "help"
        {
            let mut tokens = tokens.iter();
            let _ = tokens.next();
            self.cmdline.enqueue()?;
            self.cmdline.reset();
            self.writer.write(CRCL).await?;
            self.writer.write(START_SYNC).await?;
            if let Some(cmd_name) = tokens.next() {
                let help = C::help_for(cmd_name);
                if help.is_empty() {
                    self.writer.write_line("Unknown command".as_bytes()).await?;
                }
                self.writer.write_lines(help).await?;
            } else {
                let help = C::help_lines();
                self.writer.write_lines(help).await?;
            }
            self.writer.write(END_SYNC).await?;
            return Ok(None);
        }

        if iter.peek().is_none() {
            return Ok(None);
        }

        let result = C::parse(&mut iter);
        match result {
            Ok(command) => {
                if self.pending_offset == 0 {
                    self.writer.write(CRLFPROMPT).await?;
                    self.cmdline.enqueue()?;
                    self.cmdline.reset();
                }
                self.writer.write(CRCL).await?;
                self.pending_offset = iter.offset_in(&tokens);
                Ok(Some((command, &mut self.writer)))
            }
            Err(err) => match err.to_string::<MCB>() {
                Ok(msg) => {
                    self.writer.write(CRCL).await?;
                    self.writer.write_error(msg.as_bytes()).await?;
                    Ok(None)
                }
                Err(_) => Ok(None),
            },
        }
    }

    pub async fn finalize(&mut self) -> Result<(), RuntimeError> {
        self.redraw_prompt().await?;
        self.pending_buffer = None;
        self.pending_offset = 0;
        self.writer.flush().await
    }

    pub async fn write(&mut self, buf: &[u8]) -> Result<(), RuntimeError> {
        self.writer.write(buf).await?;
        self.writer.flush().await
    }

    pub async fn writer(&mut self) -> &mut CommandWriter<W> {
        &mut self.writer
    }
}

#[cfg(all(test, feature = "async-std"))]
mod tests {
    use super::*;
    use std::io;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tokio::io::AsyncWrite;

    /// Captures all bytes written to it. Cheap to construct, no_alloc-style backing.
    pub struct MockWriter {
        pub captured: Vec<u8>,
    }

    impl MockWriter {
        pub fn new() -> Self {
            Self {
                captured: Vec::new(),
            }
        }

        /// Returns captured output as a UTF-8 string for assertion.
        pub fn as_str(&self) -> &str {
            std::str::from_utf8(&self.captured).unwrap_or("<invalid utf-8>")
        }
    }

    impl AsyncWrite for MockWriter {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            self.captured.extend_from_slice(buf);
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    use crate::process::{CommandsParser, ProcessError};
    use crate::tokens::TokensIter;

    #[derive(Debug, PartialEq)]
    pub enum TestCommand<'a> {
        Hello,
        Echo(&'a str),
    }

    impl CommandsParser for TestCommand<'_> {
        type Match<'m> = TestCommand<'m>;

        fn parse<'p>(tokens: &mut TokensIter<'p>) -> Result<Self::Match<'p>, ProcessError<'p>> {
            // Skip leading `--`
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
    }

    #[tokio::test]
    async fn new_writes_initial_prompt() {
        let writer = MockWriter::new();
        let cli: Bareminal<TestCommand, _, 256, 4> = Bareminal::new(writer).await.unwrap();
        let captured = cli.writer.writer.as_str();
        assert!(
            captured.contains("❯ "),
            "expected prompt char in: {:?}",
            captured
        );
    }

    #[tokio::test]
    async fn typing_chars_does_not_request_command() {
        let writer = MockWriter::new();
        let mut cli: Bareminal<TestCommand, _, 256, 4> = Bareminal::new(writer).await.unwrap();
        for byte in b"hello" {
            let pending = cli.add_byte(*byte).await.unwrap();
            assert!(!pending, "no command should be pending while typing");
        }
    }

    #[tokio::test]
    async fn enter_signals_pending_commands() {
        let writer = MockWriter::new();
        let mut cli: Bareminal<TestCommand, _, 256, 4> = Bareminal::new(writer).await.unwrap();
        for byte in b"hello" {
            cli.add_byte(*byte).await.unwrap();
        }
        // Enter is 0x0d (CR) — adjust for whatever your InputParser expects.
        let pending = cli.add_byte(b'\r').await.unwrap();
        assert!(pending, "Enter should signal pending");
    }

    #[tokio::test]
    async fn next_command_returns_parsed_command() {
        let writer = MockWriter::new();
        let mut cli: Bareminal<TestCommand, _, 256, 4> = Bareminal::new(writer).await.unwrap();
        for byte in b"hello\r" {
            cli.add_byte(*byte).await.unwrap();
        }
        let Some((cmd, _writer)) = cli.next_command().await.unwrap() else {
            panic!("expected a command");
        };
        assert_eq!(cmd, TestCommand::Hello);
    }

    #[tokio::test]
    async fn next_command_returns_none_when_drained() {
        let writer = MockWriter::new();
        let mut cli: Bareminal<TestCommand, _, 256, 4> = Bareminal::new(writer).await.unwrap();
        for byte in b"hello\r" {
            cli.add_byte(*byte).await.unwrap();
        }
        let _first = cli.next_command().await.unwrap().expect("first command");
        let next = cli.next_command().await.unwrap();
        assert!(next.is_none(), "should be drained after first command");
    }

    #[tokio::test]
    async fn multiple_commands_chain() {
        let writer = MockWriter::new();
        let mut cli: Bareminal<TestCommand, _, 256, 4> = Bareminal::new(writer).await.unwrap();
        for byte in b"hello -- echo world\r" {
            cli.add_byte(*byte).await.unwrap();
        }

        let mut commands = Vec::new();
        while let Some((cmd, _)) = cli.next_command().await.unwrap() {
            commands.push(format!("{:?}", cmd));
        }

        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0], "Hello");
        assert!(commands[1].contains("Echo"));
    }

    #[tokio::test]
    async fn unknown_command_writes_error() {
        let writer = MockWriter::new();
        let mut cli: Bareminal<TestCommand, _, 256, 4> = Bareminal::new(writer).await.unwrap();
        for byte in b"unknown\r" {
            cli.add_byte(*byte).await.unwrap();
        }
        let result = cli.next_command().await.unwrap();
        assert!(result.is_none());
        let captured = cli.writer.writer.as_str();
        assert!(
            captured.contains("Unknown") || captured.contains("unknown"),
            "expected error in output: {:?}",
            captured
        );
    }

    #[tokio::test]
    async fn help_command_writes_help() {
        let writer = MockWriter::new();
        let mut cli: Bareminal<TestCommand, _, 256, 4> = Bareminal::new(writer).await.unwrap();
        for byte in b"help\r" {
            cli.add_byte(*byte).await.unwrap();
        }
        let result = cli.next_command().await.unwrap();
        assert!(result.is_none(), "help shouldn't produce a command");
        let captured = cli.writer.writer.as_str();
        assert!(
            captured.contains("Test commands"),
            "expected help in output: {:?}",
            captured
        );
    }

    #[tokio::test]
    async fn help_for_specific_command() {
        let writer = MockWriter::new();
        let mut cli: Bareminal<TestCommand, _, 256, 4> = Bareminal::new(writer).await.unwrap();
        for byte in b"help hello\r" {
            cli.add_byte(*byte).await.unwrap();
        }
        cli.next_command().await.unwrap();
        let captured = cli.writer.writer.as_str();
        assert!(
            captured.contains("hello") && captured.contains("say hello"),
            "expected per-command help: {:?}",
            captured
        );
    }

    #[tokio::test]
    async fn finalize_clears_pending_state() {
        let writer = MockWriter::new();
        let mut cli: Bareminal<TestCommand, _, 256, 4> = Bareminal::new(writer).await.unwrap();
        for byte in b"hello\r" {
            cli.add_byte(*byte).await.unwrap();
        }
        cli.next_command().await.unwrap();
        cli.finalize().await.unwrap();

        // After finalize, next_command should return None.
        let result = cli.next_command().await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn backspace_deletes_char() {
        let writer = MockWriter::new();
        let mut cli: Bareminal<TestCommand, _, 256, 4> = Bareminal::new(writer).await.unwrap();
        for byte in b"hellox" {
            cli.add_byte(*byte).await.unwrap();
        }
        cli.add_byte(0x7f).await.unwrap(); // Backspace (DEL)
        cli.add_byte(b'\r').await.unwrap();
        let Some((cmd, _)) = cli.next_command().await.unwrap() else {
            panic!("expected a command");
        };
        assert_eq!(cmd, TestCommand::Hello);
    }
}
