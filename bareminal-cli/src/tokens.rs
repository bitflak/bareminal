use core::clone::Clone;
use core::cmp::Eq;
use core::cmp::PartialEq;
use core::fmt::Debug;
use core::iter::IntoIterator;
use core::iter::Iterator;
use core::option::Option;
use core::option::Option::None;
use core::option::Option::Some;
use core::prelude::rust_2024::derive;

/// Represents a sequence of tokens packed into a single &str, where individual tokens are separated
/// by null bytes (\0). The lifetime 'a ties it to the underlying string buffer it borrows from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tokens<'a> {
    tokens: &'a str,
    empty: bool,
}

impl<'a> Tokens<'a> {
    pub fn new(input: &'a mut str) -> Self {
        let tokens = delimit_spaces(&mut *input);
        let empty = tokens.is_empty();
        Self { tokens, empty }
    }

    pub fn iter(&self) -> TokensIter<'a> {
        TokensIter::new(self.tokens, self.empty)
    }

    /// Create a new iterator starting at the given byte offset within the
    /// internal token slice. `offset` must point to the start of a token
    /// (i.e., be 0 or the byte just after a `\0` separator), or equal
    /// `self.tokens.len()` for an exhausted iterator.
    pub fn iter_from(&self, offset: usize) -> TokensIter<'a> {
        if offset >= self.tokens.len() {
            return TokensIter::new("", true);
        }
        // SAFETY: offset is at a token boundary by contract.
        let rest = unsafe { self.tokens.get_unchecked(offset..) };
        TokensIter::new(rest, rest.is_empty())
    }
}

///  As iteration proceeds, tokens is reassigned to shrinking suffixes of the original slice — each
///  call to next() lops off everything up to the next \0 and advances self.tokens past it.
#[derive(Clone, Debug)]
pub struct TokensIter<'a> {
    tokens: &'a str,
    empty: bool,
}

impl<'a> TokensIter<'a> {
    pub fn new(tokens: &'a str, empty: bool) -> Self {
        Self { tokens, empty }
    }

    pub fn peek(&self) -> Option<&'a str> {
        if self.empty {
            return None;
        }

        if let Some(pos) = self.tokens.as_bytes().iter().position(|&b| b == 0) {
            // SAFETY: range is inside boundaries
            Some(unsafe { self.tokens.get_unchecked(..pos) })
        } else {
            Some(self.tokens)
        }
    }

    pub fn peek_last(&self) -> Option<&'a str> {
        if self.empty {
            return None;
        }

        if let Some(pos) = self.tokens.as_bytes().iter().rposition(|&b| b == 0) {
            // SAFETY: pos is a valid index from rposition, pos+1 is at most len.
            // Both indices are at ASCII byte boundaries (\0), so UTF-8 stays valid.
            Some(unsafe { self.tokens.get_unchecked(pos + 1..) })
        } else {
            // No separator: only one token remains.
            Some(self.tokens)
        }
    }

    /// The byte offset into the original token slice that this iterator is
    /// currently at. Pass to `Tokens::iter_from` to resume iteration.
    pub fn offset_in(&self, source: &Tokens<'a>) -> usize {
        if self.empty {
            return source.tokens.len();
        }
        // Pointer arithmetic: self.tokens is always a suffix of source.tokens.
        let base = source.tokens.as_ptr() as usize;
        let cur = self.tokens.as_ptr() as usize;
        cur - base
    }
}

impl<'a> Iterator for TokensIter<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<<TokensIter<'a> as IntoIterator>::Item> {
        if self.empty {
            return None;
        }

        if let Some(pos) = self.tokens.as_bytes().iter().position(|&b| b == 0) {
            // SAFETY: range is inside boundaries
            let (token, rest) = unsafe {
                (
                    self.tokens.get_unchecked(..pos),
                    self.tokens.get_unchecked(pos + 1..),
                )
            };
            self.tokens = rest;
            Some(token)
        } else {
            self.empty = true;
            Some(self.tokens)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenizeError {
    UnterminatedQuote,
    DanglingEscape,
}

#[inline]
fn delimit_spaces(s: &mut str) -> &str {
    // Safe: we only read/write ASCII bytes (' ', '\t', '"', '\\', '\0') as
    // structural markers. These never appear as continuation bytes inside a
    // multi-byte UTF-8 sequence, so byte-level edits keep the slice valid UTF-8.
    let bytes = unsafe { s.as_bytes_mut() };

    let mut r = 0; // read cursor
    let mut w = 0; // write cursor (always <= r)
    let mut token_started = false;
    let mut any_token = false;

    while r < bytes.len() {
        let c = bytes[r];

        if c == b' ' || c == b'\t' {
            if token_started {
                any_token = true;
                token_started = false;
            }
            r += 1;
            continue;
        }

        if !token_started && any_token {
            bytes[w] = 0;
            w += 1;
        }
        token_started = true;

        if c == b'"' {
            r += 1;
            while r < bytes.len() {
                let q = bytes[r];
                if q == b'"' {
                    r += 1;
                    break;
                }
                if q == b'\\' {
                    r += 1;
                    if r >= bytes.len() {
                        // Trailing backslash: drop it.
                        break;
                    }
                    bytes[w] = bytes[r];
                    w += 1;
                    r += 1;
                } else {
                    bytes[w] = q;
                    w += 1;
                    r += 1;
                }
            }
        } else if c == b'\\' {
            r += 1;
            if r >= bytes.len() {
                // Trailing backslash: drop it.
                break;
            }
            bytes[w] = bytes[r];
            w += 1;
            r += 1;
        } else {
            bytes[w] = c;
            w += 1;
            r += 1;
        }
    }

    // Safe: byte-level edits above preserve UTF-8 validity (see top comment).
    unsafe { core::str::from_utf8_unchecked(&bytes[..w]) }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use heapless::String;

    #[test]
    fn token_iterator_produces_tokens() {
        let mut s = String::<64>::try_from("--a \"b c\" c=\"x b\"").unwrap();
        let mut tokens = Tokens::new(&mut s).iter();
        assert_eq!(tokens.next(), Some("--a"));
        assert_eq!(tokens.next(), Some("b c"));
        assert_eq!(tokens.next(), Some("c=x b"));
    }

    #[test]
    fn empty_input_returns_empty() {
        let mut s: heapless::String<64> = String::new();
        let out = delimit_spaces(&mut s);
        assert_eq!(out, "");
        assert_eq!(out.len(), 0);
    }

    #[test]
    fn single_word_is_unchanged() {
        let mut s = String::<64>::try_from("hello").unwrap();
        let out = delimit_spaces(&mut s);
        assert_eq!(out, "hello");
    }

    #[test]
    fn single_char_is_unchanged() {
        let mut s = String::<64>::try_from("x").unwrap();
        let out = delimit_spaces(&mut s);
        assert_eq!(out, "x");
    }

    #[test]
    fn single_space_becomes_single_nul() {
        let mut s = String::<64>::try_from("a b").unwrap();
        let out = delimit_spaces(&mut s);
        assert_eq!(out, "a\0b");
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn multiple_consecutive_spaces_collapse_to_one_nul() {
        let mut s = String::<64>::try_from("a    b").unwrap();
        let out = delimit_spaces(&mut s);
        assert_eq!(out, "a\0b");
    }

    #[test]
    fn many_words_are_nul_separated() {
        let mut s = String::<64>::try_from("one two=\"a b\" 42 n=42").unwrap();
        let out = delimit_spaces(&mut s);
        assert_eq!(out, "one\0two=a b\x0042\0n=42");
    }

    #[test]
    fn mixed_runs_of_spaces_all_collapse() {
        let mut s = String::<64>::try_from(" a b   c  ").unwrap();
        let out = delimit_spaces(&mut s);
        assert_eq!(out, "a\0b\0c");
    }

    #[test]
    fn only_spaces_becomes_empty() {
        let mut s = String::<64>::try_from("     ").unwrap();
        let out = delimit_spaces(&mut s);
        assert_eq!(out, "");
        assert_eq!(out.len(), 0);
    }

    #[test]
    fn double_quotes_are_stripped() {
        let mut s = String::<64>::try_from("\"hello\"").unwrap();
        let out = delimit_spaces(&mut s);
        assert_eq!(out, "hello");
    }

    #[test]
    fn single_quotes_are_preserved() {
        let mut s = String::<64>::try_from("\"foo's bar\"").unwrap();
        let out = delimit_spaces(&mut s);
        assert_eq!(out, "foo's bar");
    }

    #[test]
    fn only_quotes_becomes_empty() {
        let mut s = String::<64>::try_from("\"\"\"").unwrap();
        let out = delimit_spaces(&mut s);
        assert_eq!(out, "");
    }

    #[test]
    fn quotes_around_spaces_do_not_preserve_spaces() {
        let mut s = String::<64>::try_from("\"hello world\"").unwrap();
        let out = delimit_spaces(&mut s);
        assert_eq!(out, "hello world");
    }

    #[test]
    fn quotes_adjacent_to_words_do_not_split_them() {
        let mut s = String::<64>::try_from("foo\"bar\"baz").unwrap();
        let out = delimit_spaces(&mut s);
        assert_eq!(out, "foobarbaz");
    }

    #[test]
    fn quote_then_space_preserves_space() {
        let mut s = String::<64>::try_from("a=a\" b").unwrap();
        let out = delimit_spaces(&mut s);
        assert_eq!(out, "a=a b");
    }

    #[test]
    fn quote_between_spaces_does_not_break_space_run() {
        let mut s = String::<64>::try_from("a \" b").unwrap();
        let out = delimit_spaces(&mut s);
        assert_eq!(out, "a\0 b");
    }

    #[test]
    fn tab_is_treated_as_space_character() {
        // Only ASCII space (0x20) is a delimiter; tabs pass through.
        let mut s = String::<64>::try_from("a\tb").unwrap();
        let out = delimit_spaces(&mut s);
        assert_eq!(out, "a\0b");
    }

    #[test]
    fn newline_is_treated_as_regular_character() {
        let mut s = String::<64>::try_from("a\nb").unwrap();
        let out = delimit_spaces(&mut s);
        assert_eq!(out, "a\nb");
    }

    #[test]
    fn preserves_multibyte_utf8_in_words() {
        let mut s = String::<64>::try_from("héllo wörld").unwrap();
        let out = delimit_spaces(&mut s);
        assert_eq!(out, "héllo\0wörld");
    }

    #[test]
    fn preserves_emoji_across_collapse() {
        let mut s = String::<64>::try_from("🦀   rust   🚀").unwrap();
        let out = delimit_spaces(&mut s);
        assert_eq!(out, "🦀\0rust\0🚀");
    }
}
