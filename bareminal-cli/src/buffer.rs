use crate::{tokens::Tokens, utf::Utf8Accumulator};
use core::clone::Clone;
use core::default::Default;
use core::iter::Iterator;
use core::ops::Fn;
use core::option::Option;
use core::option::Option::None;
use core::option::Option::Some;
use core::prelude::rust_2024::derive;
use core::result::Result;
use core::result::Result::Err;
use core::result::Result::Ok;
use core::str::FromStr;

use crate::process::RuntimeError;

#[derive(Clone)]
pub struct Buffer<const MAX_BUFFER: usize> {
    buffer: [u8; MAX_BUFFER],
    size: usize,
}

impl<const MAX_BUFFER: usize> Buffer<MAX_BUFFER> {
    pub fn new() -> Self {
        Self {
            buffer: [0; MAX_BUFFER],
            size: 0,
        }
    }

    /// Attention: Each bytes array expected to be a single char
    pub fn write_char(&mut self, bytes: &[u8]) -> bool {
        if self.size + bytes.len() <= MAX_BUFFER {
            for &byte in bytes {
                self.buffer[self.size] = byte;
                self.size += 1;
            }
            return true;
        }
        false
    }

    pub fn write_str(&mut self, str: &str) -> bool {
        // Pre-check: does the whole string fit?
        if self.size + str.len() > MAX_BUFFER {
            return false;
        }

        let start_size = self.size;
        for (i, c) in str.char_indices() {
            let end = i + c.len_utf8();
            self.write_char(&str.as_bytes()[i..end]);
        }
        start_size != self.size || str.is_empty()
    }

    pub fn reset(&mut self) {
        self.size = 0;
    }

    pub fn chars_count(&self) -> usize {
        // # Safety
        // The caller must ensure `bytes` contains valid UTF-8.
        unsafe {
            core::str::from_utf8_unchecked(&self.buffer[..self.size])
                .chars()
                .count()
        }
    }

    /// Each bytes array expected to have exactly one vald utf8 charcter
    pub fn write_at_char_pos(&mut self, bytes: &[u8], pos: usize) -> bool {
        if self.size + bytes.len() > MAX_BUFFER {
            return false;
        }

        let byte_idx = match self.byte_index(pos) {
            Some(idx) => idx,
            None if pos == self.chars_count() => self.size,
            None => return false,
        };

        self.buffer
            .copy_within(byte_idx..self.size, byte_idx + bytes.len());
        for (i, &b) in bytes.iter().enumerate() {
            self.buffer[byte_idx + i] = b;
        }

        self.size += bytes.len();

        true
    }

    pub fn delete_char(&mut self, pos: usize) -> bool {
        if pos < 1 {
            return false;
        }

        let char_pos = self.byte_index(pos - 1);

        let next_pos = if let Some(idx) = char_pos {
            unsafe {
                let text = self.buffer.get_unchecked(idx..self.size);
                byte_index(text, 1).map(|p| p + idx)
            }
        } else {
            None
        };

        match (char_pos, next_pos) {
            (Some(cursor), None) => {
                self.size = cursor;
                true
            }
            (Some(cursor), Some(next)) => {
                self.buffer.copy_within(next..self.size, cursor);
                self.size -= next - cursor;
                true
            }
            _ => false,
        }
    }

    /// # Safety
    /// The caller must ensure `bytes` contains valid UTF-8.
    pub fn byte_index(&self, pos: usize) -> Option<usize> {
        byte_index(&self.buffer[..self.size], pos)
    }

    pub fn as_tokens(&mut self) -> Option<Tokens<'_>> {
        if self.size > 0 {
            unsafe {
                Some(Tokens::new(core::str::from_utf8_unchecked_mut(
                    &mut self.buffer[..self.size],
                )))
            }
        } else {
            None
        }
    }

    pub fn as_str(&mut self) -> Option<&str> {
        if self.size > 0 {
            unsafe {
                Some(core::str::from_utf8_unchecked_mut(
                    &mut self.buffer[..self.size],
                ))
            }
        } else {
            None
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        if self.size > 0 {
            Some(&self.buffer[..self.size])
        } else {
            None
        }
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
    pub fn write_table(&mut self, headers: &[&str], rows: &[&[&str]]) -> Result<(), RuntimeError> {
        let cols = headers.len();

        for row in rows {
            if row.len() != cols {
                return Err(RuntimeError::ColumnMismatch);
            }
        }

        let col_width = |c: usize| -> usize {
            let mut w = headers[c].chars().count();
            for row in rows {
                let cw = row[c].chars().count();
                if cw > w {
                    w = cw;
                }
            }
            w
        };

        self.write_table_row(headers, &col_width)?;
        self.write_table_separator(cols, &col_width)?;
        for row in rows {
            self.write_table_row(row, &col_width)?;
        }

        Ok(())
    }

    fn put(&mut self, s: &str) -> Result<(), RuntimeError> {
        if self.write_str(s) || s.is_empty() {
            Ok(())
        } else {
            Err(RuntimeError::BufferTooSmall)
        }
    }

    fn write_table_row<F>(&mut self, cells: &[&str], col_width: &F) -> Result<(), RuntimeError>
    where
        F: Fn(usize) -> usize,
    {
        self.put("|")?;
        for (i, cell) in cells.iter().enumerate() {
            let width = col_width(i);
            let cell_chars = cell.chars().count();
            self.put(" ")?;
            self.put(cell)?;
            for _ in cell_chars..width {
                self.put(" ")?;
            }
            self.put(" |")?;
        }
        self.put("\r\n")?;
        Ok(())
    }

    fn write_table_separator<F>(&mut self, cols: usize, col_width: &F) -> Result<(), RuntimeError>
    where
        F: Fn(usize) -> usize,
    {
        self.put("|")?;
        for c in 0..cols {
            let width = col_width(c);
            for _ in 0..(width + 2) {
                self.put("-")?;
            }
            self.put("|")?;
        }
        self.put("\r\n")?;
        Ok(())
    }

    /// Format a key/value list and append it to the buffer.
    ///
    /// Each entry must have exactly two cells: a name and a value.
    /// Names are padded so all values align to the same column.
    ///
    /// If `enumerate` is true, each row is prefixed with its 1-based index
    /// (e.g. ` 1. name:   value`). Index numbers are right-aligned so the
    /// names stay vertically aligned regardless of how many entries there are.
    pub fn write_list(&mut self, entries: &[&[&str]], enumerate: bool) -> Result<(), RuntimeError> {
        for entry in entries {
            if entry.len() != 2 {
                return Err(RuntimeError::ColumnMismatch);
            }
        }

        let name_width = entries
            .iter()
            .map(|e| e[0].chars().count())
            .max()
            .unwrap_or(0);

        // Width of the index column (number of digits in the largest index).
        // 0 if not enumerating.
        let index_width = if enumerate {
            digit_count(entries.len())
        } else {
            0
        };

        for (i, entry) in entries.iter().enumerate() {
            let name = entry[0];
            let value = entry[1];
            let name_chars = name.chars().count();

            if enumerate {
                let index = i + 1;
                let idx_chars = digit_count(index);
                // Right-align the index by padding spaces in front.
                for _ in idx_chars..index_width {
                    self.put(" ")?;
                }
                self.write_usize(index)?;
                self.put(". ")?;
            }

            self.put(name)?;
            self.put(":")?;
            for _ in name_chars..name_width {
                self.put(" ")?;
            }
            self.put("  ")?;
            self.put(value)?;
            self.put("\r\n")?;
        }

        Ok(())
    }

    fn write_usize(&mut self, mut n: usize) -> Result<(), RuntimeError> {
        if n == 0 {
            return self.put("0");
        }
        let mut tmp = [0u8; 20]; // enough for usize on any target
        let mut i = tmp.len();
        while n > 0 {
            i -= 1;
            tmp[i] = b'0' + (n % 10) as u8;
            n /= 10;
        }
        // SAFETY: only ASCII digits written.
        let s = unsafe { core::str::from_utf8_unchecked(&tmp[i..]) };
        self.put(s)
    }

    /// Serialize a value as pretty-printed JSON and append it to the buffer.
    /// Uses 2-space indentation.
    pub fn write_json<T: serde::Serialize>(
        &mut self,
        value: &T,
        pretty: bool,
    ) -> Result<(), RuntimeError> {
        let mut scratch = [0u8; MAX_BUFFER];
        let len = serde_json_core::to_slice(value, &mut scratch)
            .map_err(|_| RuntimeError::BufferTooSmall)?;
        let json = &scratch[..len];

        if pretty {
            self.pretty_print_json(json)
        } else {
            // SAFETY: serde_json_core produces valid UTF-8 JSON.
            let s = unsafe { core::str::from_utf8_unchecked(json) };
            if s.is_empty() || Buffer::write_str(self, s) {
                Ok(())
            } else {
                Err(RuntimeError::BufferTooSmall)
            }
        }
    }

    /// Walk a compact JSON byte string and append it pretty-printed.
    fn pretty_print_json(&mut self, json: &[u8]) -> Result<(), RuntimeError> {
        let mut depth: usize = 0;
        let mut in_string = false;
        let mut escape = false;
        let mut i = 0;

        while i < json.len() {
            let b = json[i];

            if in_string {
                self.put_byte(b)?;
                if escape {
                    escape = false;
                } else if b == b'\\' {
                    escape = true;
                } else if b == b'"' {
                    in_string = false;
                }
                i += 1;
                continue;
            }

            match b {
                b'"' => {
                    in_string = true;
                    self.put_byte(b)?;
                }
                b'{' | b'[' => {
                    let close = if b == b'{' { b'}' } else { b']' };
                    // Empty container: emit `{}` or `[]` on a single line.
                    if json.get(i + 1).copied() == Some(close) {
                        self.put_byte(b)?;
                        self.put_byte(close)?;
                        i += 2;
                        continue;
                    }
                    self.put_byte(b)?;
                    depth += 1;
                    self.put("\r\n")?;
                    self.write_indent(depth)?;
                }
                b'}' | b']' => {
                    depth = depth.saturating_sub(1);
                    self.put("\r\n")?;
                    self.write_indent(depth)?;
                    self.put_byte(b)?;
                }
                b',' => {
                    self.put_byte(b)?;
                    self.put("\r\n")?;
                    self.write_indent(depth)?;
                }
                b':' => {
                    self.put(": ")?;
                }
                _ => self.put_byte(b)?,
            }
            i += 1;
        }
        Ok(())
    }

    fn write_indent(&mut self, depth: usize) -> Result<(), RuntimeError> {
        for _ in 0..depth {
            self.put("  ")?;
        }
        Ok(())
    }

    fn put_byte(&mut self, b: u8) -> Result<(), RuntimeError> {
        // SAFETY: only called with bytes from valid UTF-8 JSON output.
        let s = unsafe { core::str::from_utf8_unchecked(core::slice::from_ref(&b)) };
        if Buffer::write_str(self, s) {
            Ok(())
        } else {
            Err(RuntimeError::BufferTooSmall)
        }
    }
}

impl<const MAX_BUFFER: usize> Default for Buffer<MAX_BUFFER> {
    fn default() -> Self {
        Self {
            buffer: [0; MAX_BUFFER],
            size: 0,
        }
    }
}

impl<const MAX_BUFFER: usize> FromStr for Buffer<MAX_BUFFER> {
    type Err = ();

    fn from_str(str: &str) -> Result<Self, ()> {
        let mut buffer = Buffer::<MAX_BUFFER>::new();
        let mut acc = Utf8Accumulator::new();

        for byte in str.as_bytes().iter() {
            if let Some(char) = acc.push(*byte) {
                buffer.write_char(char.as_bytes());
            }
        }

        Ok(buffer)
    }
}

pub fn byte_index(bytes: &[u8], pos: usize) -> Option<usize> {
    // SAFETY: caller guarantees valid UTF-8
    let s = unsafe { core::str::from_utf8_unchecked(bytes) };

    s.char_indices()
        .nth(pos)
        .map(|(idx, _)| Some(idx))
        .unwrap_or(None)
}

impl<const MAX_BUFFER: usize> core::fmt::Write for Buffer<MAX_BUFFER> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        if s.is_empty() || self.write_str(s) {
            Ok(())
        } else {
            Err(core::fmt::Error)
        }
    }
}

fn digit_count(mut n: usize) -> usize {
    if n == 0 {
        return 1;
    }
    let mut count = 0;
    while n > 0 {
        count += 1;
        n /= 10;
    }
    count
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn ascii() {
        let s = "hello".as_bytes();
        assert_eq!(byte_index(s, 0), Some(0)); // after 'h'
        assert_eq!(byte_index(s, 2), Some(2)); // after 'l'
        assert_eq!(byte_index(s, 4), Some(4)); // after 'o' (end)
    }

    #[test]
    fn multibyte() {
        // 'é' = 2 bytes, '日' = 3 bytes, '🦀' = 4 bytes
        let s = "aé日🦀b".as_bytes();
        assert_eq!(byte_index(s, 0), Some(0)); // after 'a'
        assert_eq!(byte_index(s, 1), Some(1)); // after 'é'
        assert_eq!(byte_index(s, 2), Some(3)); // after '日'
        assert_eq!(byte_index(s, 3), Some(6)); // after '🦀'
        assert_eq!(byte_index(s, 4), Some(10)); // after 'b'
    }

    #[test]
    fn out_of_bounds_returns_none() {
        let s = "abc".as_bytes();
        assert_eq!(byte_index(s, 100), None);
    }

    #[test]
    fn write_at_char_pos_multibyte() {
        let mut buffer = Buffer::<16>::new();
        // [a, 😀, b] 1+4+1 = 6 bytes
        buffer.write_char("a\u{1F600}b".as_bytes());
        assert!(buffer.write_at_char_pos(b"X", 1));
        assert_eq!(buffer.as_bytes().unwrap(), "aX\u{1F600}b".as_bytes());
        assert!(buffer.write_at_char_pos(b"Y", 2));
        assert_eq!(buffer.as_bytes().unwrap(), "aXY\u{1F600}b".as_bytes());
    }

    #[test]
    fn write_at_char_pos_at_the_end() {
        let mut buffer = Buffer::<16>::new();
        buffer.write_char(b"abc");
        assert!(buffer.write_at_char_pos(b"d", 3));
        assert_eq!(buffer.as_bytes().unwrap(), b"abcd");
    }

    #[test]
    fn delete_char_test() {
        let mut buffer = Buffer::<16>::new();
        buffer.write_char("a\u{1F600}b".as_bytes()); // [a, 😀, b]
        assert!(buffer.delete_char(1));
        assert_eq!(buffer.as_bytes().unwrap(), "\u{1F600}b".as_bytes());
        assert!(buffer.delete_char(1));
        assert_eq!(buffer.as_bytes().unwrap(), b"b");
        assert!(buffer.delete_char(1));
        assert_eq!(buffer.as_bytes().unwrap_or(&[]), b"");
    }

    #[test]
    fn delete_char_out_of_bounds() {
        let mut buffer = Buffer::<16>::new();
        buffer.write_char(b"abc");
        assert!(!buffer.delete_char(0));
        assert!(!buffer.delete_char(4));
    }

    /// Helper: render the buffer's contents as a `&str` for assertions.
    fn as_str<const N: usize>(buf: &Buffer<N>) -> &str {
        let bytes = buf.as_bytes().expect("buffer should not be empty");
        core::str::from_utf8(bytes).expect("buffer should be valid UTF-8")
    }

    #[test]
    fn writes_simple_table() {
        let mut buf: Buffer<256> = Buffer::new();
        let headers = ["Name", "Age"];
        let rows: &[&[&str]] = &[&["Alice", "30"], &["Bob", "7"]];

        buf.write_table(&headers, rows).unwrap();

        let expected = "\
| Name  | Age |\r\n\
|-------|-----|\r\n\
| Alice | 30  |\r\n\
| Bob   | 7   |\r\n";
        assert_eq!(as_str(&buf), expected);
    }

    #[test]
    fn pads_to_widest_cell_in_each_column() {
        let mut buf: Buffer<256> = Buffer::new();
        let headers = ["A", "B"];
        let rows: &[&[&str]] = &[&["short", "x"], &["xx", "muchlonger"]];

        buf.write_table(&headers, rows).unwrap();

        let expected = "\
| A     | B          |\r\n\
|-------|------------|\r\n\
| short | x          |\r\n\
| xx    | muchlonger |\r\n";
        assert_eq!(as_str(&buf), expected);
    }

    #[test]
    fn header_can_be_widest() {
        let mut buf: Buffer<256> = Buffer::new();
        let headers = ["LongHeader", "X"];
        let rows: &[&[&str]] = &[&["a", "b"]];

        buf.write_table(&headers, rows).unwrap();

        let expected = "\
| LongHeader | X |\r\n\
|------------|---|\r\n\
| a          | b |\r\n";
        assert_eq!(as_str(&buf), expected);
    }

    #[test]
    fn handles_no_rows() {
        let mut buf: Buffer<128> = Buffer::new();
        let headers = ["Col1", "Col2"];
        let rows: &[&[&str]] = &[];

        buf.write_table(&headers, rows).unwrap();

        let expected = "\
| Col1 | Col2 |\r\n\
|------|------|\r\n";
        assert_eq!(as_str(&buf), expected);
    }

    #[test]
    fn handles_empty_cells() {
        let mut buf: Buffer<128> = Buffer::new();
        let headers = ["A", "B"];
        let rows: &[&[&str]] = &[&["", ""], &["x", ""]];

        buf.write_table(&headers, rows).unwrap();

        let expected = "\
| A | B |\r\n\
|---|---|\r\n\
|   |   |\r\n\
| x |   |\r\n";
        assert_eq!(as_str(&buf), expected);
    }

    #[test]
    fn aligns_multibyte_utf8_by_character_count() {
        let mut buf: Buffer<256> = Buffer::new();
        let headers = ["City", "Note"];
        // "München" and "Berlin" both have 6 chars but different byte lengths.
        let rows: &[&[&str]] = &[&["München", "ok"], &["Berlin", "fine"]];

        buf.write_table(&headers, rows).unwrap();

        // Column 1 width = max(4, 7, 6) = 7 chars
        // Column 2 width = max(4, 2, 4) = 4 chars
        let expected = "\
| City    | Note |\r\n\
|---------|------|\r\n\
| München | ok   |\r\n\
| Berlin  | fine |\r\n";
        assert_eq!(as_str(&buf), expected);
    }

    #[test]
    fn rejects_row_with_too_few_columns() {
        let mut buf: Buffer<256> = Buffer::new();
        let headers = ["A", "B", "C"];
        let rows: &[&[&str]] = &[&["1", "2"]]; // only 2 cells

        let err = buf.write_table(&headers, rows).unwrap_err();
        assert_eq!(err, RuntimeError::ColumnMismatch);
    }

    #[test]
    fn rejects_row_with_too_many_columns() {
        let mut buf: Buffer<256> = Buffer::new();
        let headers = ["A", "B"];
        let rows: &[&[&str]] = &[&["1", "2", "3"]];

        let err = buf.write_table(&headers, rows).unwrap_err();
        assert_eq!(err, RuntimeError::ColumnMismatch);
    }

    #[test]
    fn validation_happens_before_writing() {
        // If row validation fails, the buffer should remain untouched.
        let mut buf: Buffer<256> = Buffer::new();
        let headers = ["A", "B"];
        let rows: &[&[&str]] = &[&["1"]];

        let _ = buf.write_table(&headers, rows);
        assert!(
            buf.as_bytes().is_none(),
            "buffer should be empty on validation error"
        );
    }

    #[test]
    fn returns_buffer_too_small_when_capacity_exceeded() {
        // 16 bytes can't hold even the header of this table.
        let mut buf: Buffer<16> = Buffer::new();
        let headers = ["LongerHeader", "AnotherOne"];
        let rows: &[&[&str]] = &[&["foo", "bar"]];

        let err = buf.write_table(&headers, rows).unwrap_err();
        assert_eq!(err, RuntimeError::BufferTooSmall);
    }

    #[test]
    fn appends_to_existing_buffer_contents() {
        // write_table appends; it does not reset.
        let mut buf: Buffer<256> = Buffer::new();
        buf.write_str("prefix:\r\n");

        let headers = ["A"];
        let rows: &[&[&str]] = &[&["1"]];
        buf.write_table(&headers, rows).unwrap();

        let expected = "\
prefix:\r\n\
| A |\r\n\
|---|\r\n\
| 1 |\r\n";
        assert_eq!(as_str(&buf), expected);
    }

    #[test]
    fn every_row_ends_with_crlf() {
        let mut buf: Buffer<256> = Buffer::new();
        let headers = ["A", "B"];
        let rows: &[&[&str]] = &[&["1", "2"], &["3", "4"]];

        buf.write_table(&headers, rows).unwrap();

        let s = as_str(&buf);
        // Header + separator + 2 data rows = 4 lines, each ending in \r\n.
        assert_eq!(s.matches("\r\n").count(), 4);
        assert!(s.ends_with("\r\n"));
        // No bare LFs without a preceding CR.
        assert!(!s.contains("\n\n"));
    }

    #[test]
    fn single_column_table() {
        let mut buf: Buffer<128> = Buffer::new();
        let headers = ["Item"];
        let rows: &[&[&str]] = &[&["apple"], &["fig"]];

        buf.write_table(&headers, rows).unwrap();

        let expected = "\
| Item  |\r\n\
|-------|\r\n\
| apple |\r\n\
| fig   |\r\n";
        assert_eq!(as_str(&buf), expected);
    }

    #[test]
    fn writes_simple_list() {
        let mut buf: Buffer<128> = Buffer::new();
        let entries: &[&[&str]] = &[&["name", "value"], &["name2", "value"]];

        buf.write_list(entries, false).unwrap();

        let expected = "\
name:   value\r\n\
name2:  value\r\n";
        assert_eq!(as_str(&buf), expected);
    }

    #[test]
    fn list_aligns_multibyte_names() {
        let mut buf: Buffer<128> = Buffer::new();
        let entries: &[&[&str]] = &[&["München", "city"], &["id", "42"]];

        buf.write_list(entries, false).unwrap();

        let expected = "\
München:  city\r\n\
id:       42\r\n";
        assert_eq!(as_str(&buf), expected);
    }

    #[test]
    fn list_rejects_wrong_shape() {
        let mut buf: Buffer<128> = Buffer::new();
        let entries: &[&[&str]] = &[&["only-one"]];

        let err = buf.write_list(entries, false).unwrap_err();
        assert_eq!(err, RuntimeError::ColumnMismatch);
    }

    use serde::Serialize;

    #[derive(Serialize)]
    struct SimpleConfig {
        name: &'static str,
        port: u16,
        enabled: bool,
    }

    #[derive(Serialize)]
    struct NestedConfig {
        name: &'static str,
        network: Network,
        tags: [&'static str; 2],
    }

    #[derive(Serialize)]
    struct Network {
        host: &'static str,
        port: u16,
    }

    #[test]
    fn compact_simple_struct() {
        let cfg = SimpleConfig {
            name: "sensor",
            port: 8080,
            enabled: true,
        };
        let mut buf: Buffer<256> = Buffer::new();
        buf.write_json(&cfg, false).unwrap();

        assert_eq!(
            as_str(&buf),
            r#"{"name":"sensor","port":8080,"enabled":true}"#
        );
    }

    #[test]
    fn pretty_simple_struct() {
        let cfg = SimpleConfig {
            name: "sensor",
            port: 8080,
            enabled: true,
        };
        let mut buf: Buffer<256> = Buffer::new();
        buf.write_json(&cfg, true).unwrap();

        let expected = "\
{\r\n  \"name\": \"sensor\",\r\n  \"port\": 8080,\r\n  \"enabled\": true\r\n}";
        assert_eq!(as_str(&buf), expected);
    }

    #[test]
    fn pretty_nested_struct() {
        let cfg = NestedConfig {
            name: "sensor",
            network: Network {
                host: "localhost",
                port: 8080,
            },
            tags: ["primary", "outdoor"],
        };
        let mut buf: Buffer<512> = Buffer::new();
        buf.write_json(&cfg, true).unwrap();

        let expected = "\
{\r\n  \
\"name\": \"sensor\",\r\n  \
\"network\": {\r\n    \
\"host\": \"localhost\",\r\n    \
\"port\": 8080\r\n  \
},\r\n  \
\"tags\": [\r\n    \
\"primary\",\r\n    \
\"outdoor\"\r\n  \
]\r\n\
}";
        assert_eq!(as_str(&buf), expected);
    }

    #[test]
    fn pretty_keeps_empty_object_inline() {
        #[derive(Serialize)]
        struct WithEmpty {
            data: Empty,
            n: i32,
        }
        #[derive(Serialize)]
        struct Empty {}

        let cfg = WithEmpty {
            data: Empty {},
            n: 1,
        };
        let mut buf: Buffer<128> = Buffer::new();
        buf.write_json(&cfg, true).unwrap();

        let expected = "{\r\n  \"data\": {},\r\n  \"n\": 1\r\n}";
        assert_eq!(as_str(&buf), expected);
    }

    #[test]
    fn pretty_keeps_empty_array_inline() {
        #[derive(Serialize)]
        struct WithList {
            items: [i32; 0],
            n: i32,
        }

        let cfg = WithList { items: [], n: 1 };
        let mut buf: Buffer<128> = Buffer::new();
        buf.write_json(&cfg, true).unwrap();

        let expected = "{\r\n  \"items\": [],\r\n  \"n\": 1\r\n}";
        assert_eq!(as_str(&buf), expected);
    }

    #[test]
    fn strings_containing_structural_chars_are_preserved() {
        // The string contains { , : ] " — none of these should trigger formatting.
        #[derive(Serialize)]
        struct Tricky {
            note: &'static str,
        }

        let cfg = Tricky {
            note: r#"has {braces}, "quotes", and: colons]"#,
        };
        let mut buf: Buffer<256> = Buffer::new();
        buf.write_json(&cfg, true).unwrap();

        // Inside the JSON string, structural characters must appear verbatim
        // (with quotes escaped by serde_json_core).
        let expected = "{\r\n  \"note\": \"has {braces}, \\\"quotes\\\", and: colons]\"\r\n}";
        assert_eq!(as_str(&buf), expected);
    }

    #[test]
    fn escaped_backslash_does_not_break_string_tracking() {
        // Trailing `\\` in the source becomes `\\\\` in JSON. The pretty-printer
        // must not interpret the second backslash as escaping the closing quote.
        #[derive(Serialize)]
        struct Path {
            p: &'static str,
            n: i32,
        }

        let cfg = Path { p: r"C:\\", n: 7 };
        let mut buf: Buffer<128> = Buffer::new();
        buf.write_json(&cfg, true).unwrap();

        let expected = "{\r\n  \"p\": \"C:\\\\\\\\\",\r\n  \"n\": 7\r\n}";
        assert_eq!(as_str(&buf), expected);
    }

    #[test]
    fn compact_and_pretty_parse_to_same_value() {
        // Sanity check: both forms are valid JSON representing the same data.
        let cfg = SimpleConfig {
            name: "x",
            port: 1,
            enabled: false,
        };

        let mut compact_buf: Buffer<128> = Buffer::new();
        let mut pretty_buf: Buffer<128> = Buffer::new();
        compact_buf.write_json(&cfg, false).unwrap();
        pretty_buf.write_json(&cfg, true).unwrap();

        // Round-trip both through serde_json_core back to a struct.
        let compact_bytes = compact_buf.as_bytes().unwrap();
        let pretty_bytes = pretty_buf.as_bytes().unwrap();

        let (parsed_compact, _): (SimpleConfigOwned, _) =
            serde_json_core::from_slice(compact_bytes).unwrap();
        let (parsed_pretty, _): (SimpleConfigOwned, _) =
            serde_json_core::from_slice(pretty_bytes).unwrap();

        assert_eq!(parsed_compact, parsed_pretty);
    }

    // Owned variant for parsing back (heapless string would also work).
    #[derive(serde::Deserialize, PartialEq, Debug)]
    struct SimpleConfigOwned {
        name: heapless::String<32>,
        port: u16,
        enabled: bool,
    }

    #[test]
    fn appends_json_to_existing_buffer_contents() {
        let mut buf: Buffer<256> = Buffer::new();
        buf.write_str("prefix: ");

        let cfg = SimpleConfig {
            name: "x",
            port: 1,
            enabled: true,
        };
        buf.write_json(&cfg, false).unwrap();

        assert_eq!(
            as_str(&buf),
            r#"prefix: {"name":"x","port":1,"enabled":true}"#
        );
    }

    #[test]
    fn json_returns_buffer_too_small_when_capacity_exceeded() {
        let cfg = SimpleConfig {
            name: "this name is far too long to fit",
            port: 8080,
            enabled: true,
        };
        let mut buf: Buffer<16> = Buffer::new();

        let err = buf.write_json(&cfg, false).unwrap_err();
        assert_eq!(err, RuntimeError::BufferTooSmall);
    }

    #[test]
    fn handles_signed_negative_and_floats() {
        #[derive(Serialize)]
        struct Mixed {
            i: i32,
            f: f32,
        }

        let cfg = Mixed { i: -42, f: 1.5 };
        let mut buf: Buffer<128> = Buffer::new();
        buf.write_json(&cfg, false).unwrap();

        // serde_json_core formats f32 as "1.5" exactly here.
        assert_eq!(as_str(&buf), r#"{"i":-42,"f":1.5}"#);
    }

    #[test]
    fn handles_option_some_and_none() {
        #[derive(Serialize)]
        struct WithOpt {
            a: Option<i32>,
            b: Option<i32>,
        }

        let cfg = WithOpt {
            a: Some(7),
            b: None,
        };
        let mut buf: Buffer<128> = Buffer::new();
        buf.write_json(&cfg, false).unwrap();

        assert_eq!(as_str(&buf), r#"{"a":7,"b":null}"#);
    }
}
