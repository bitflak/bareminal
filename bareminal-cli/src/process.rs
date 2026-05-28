use core::cmp::Eq;
use core::cmp::PartialEq;
use core::convert::From;
use core::fmt::Debug;
use core::iter::Iterator;
use core::option::Option;
use core::option::Option::None;
use core::option::Option::Some;
use core::prelude::rust_2024::derive;
use core::result::Result;

use crate::tokens::TokensIter;

#[derive(Debug, PartialEq)]
pub enum ProcessError<'a> {
    Empty,
    Unknown,
    MissingValue(&'a str),
    MissingRequired(&'a str),
    MissingFlag(&'a str),
    UnknownFlag(&'a str),
    UnknownArg(&'a str),
    InvalidFormat(&'a str),
    InvalidValue((&'a str, &'a str, &'a str)),
    Duplicate(&'a str),
    OutOfRange((&'a str, &'a str, &'a str, &'a str)),
    NotInSet((&'a str, &'a str, &'a [&'a str])),
}

impl<'a> ProcessError<'a> {
    pub fn to_string<const MAX_ERROR_BUFFER: usize>(
        &self,
    ) -> core::result::Result<heapless::String<MAX_ERROR_BUFFER>, core::fmt::Error> {
        match self {
            ProcessError::Empty => heapless::format!("Missing input value"),
            ProcessError::Unknown => {
                heapless::format!("Unknown command, run help for more information")
            }
            ProcessError::OutOfRange((_, _, min, max)) => {
                heapless::format!("Out of range. min: {} max: {}", min, max)
            }
            ProcessError::NotInSet((_, _, set_array)) => {
                heapless::format!("Allowed values: {:?}", set_array)
            }
            ProcessError::MissingValue(key) => {
                heapless::format!("Missing value for argument '--{}'", key)
            }
            ProcessError::MissingRequired(name) => {
                heapless::format!("Missing required argument '--{}'", name)
            }
            ProcessError::MissingFlag(name) => {
                heapless::format!("Missing required flag '{}'", name)
            }
            ProcessError::UnknownFlag(name) => {
                heapless::format!("Unknown flag '{}'", name)
            }
            ProcessError::UnknownArg(key) => heapless::format!("unknown argument '--{}'", key),
            ProcessError::InvalidFormat(token) => heapless::format!(
                "Invalid argument format: expected '--name', got '{}'",
                token
            ),
            ProcessError::InvalidValue((key, format, token)) => {
                heapless::format!(
                    "Invalid value format for {}: expected {}, got '{}'",
                    key,
                    format,
                    token
                )
            }
            ProcessError::Duplicate(name) => {
                heapless::format!("Duplicate argument '--{}'", name)
            }
        }
    }
}

pub trait CommandsParser {
    type Match<'m>: core::fmt::Debug;
    fn autocomplete(name: &str) -> Option<&'static str>;
    fn parse<'p>(tokens: &mut TokensIter<'p>) -> Result<Self::Match<'p>, ProcessError<'p>>;
    fn help() -> &'static [HelpSegment];
    fn help_for(name: &str) -> &'static [HelpSegment];
    fn help_lines() -> HelpIter {
        HelpIter::single(Self::help())
    }
}

/// Maximum rendered width of a single help line (used for runtime
/// rendering of `one_of` segments whose values are not known at
/// macro-expansion time). Lines that would overflow are truncated.
pub const MAX_HELP_LINE: usize = 256;

/// A help line as stored in `&'static` per-command help tables.
///
/// `Static` is the common case (compile-time known). `OneOf` carries a
/// runtime slice of allowed values that the iterator formats into a
/// single line as `<prefix>v1, v2, ...]`.
#[derive(Debug, Clone, Copy)]
pub enum HelpSegment {
    Static(&'static str),
    OneOf {
        prefix: &'static str,
        items: &'static [&'static str],
    },
}

/// One yielded help line — borrowed for static segments, owned for
/// runtime-rendered segments.
#[derive(Debug)]
pub enum HelpLine {
    Static(&'static str),
    Owned(heapless::String<MAX_HELP_LINE>),
}

impl AsRef<str> for HelpLine {
    fn as_ref(&self) -> &str {
        match self {
            HelpLine::Static(s) => s,
            HelpLine::Owned(s) => s.as_str(),
        }
    }
}

fn render_one_of(prefix: &str, items: &[&str]) -> heapless::String<MAX_HELP_LINE> {
    let mut out: heapless::String<MAX_HELP_LINE> = heapless::String::new();
    let _ = out.push_str(prefix);
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            let _ = out.push_str(", ");
        }
        let _ = out.push_str(item);
    }
    let _ = out.push(']');
    out
}

pub enum HelpIter {
    Single {
        segments: &'static [HelpSegment],
        idx: usize,
    },
    Multi {
        sections: &'static [(&'static str, &'static [HelpSegment])],
        section_idx: usize,
        seg_idx: usize,
        header_emitted: bool,
        blank_pending: bool,
    },
}

impl HelpIter {
    pub const fn single(segments: &'static [HelpSegment]) -> Self {
        HelpIter::Single { segments, idx: 0 }
    }

    pub const fn multi(sections: &'static [(&'static str, &'static [HelpSegment])]) -> Self {
        HelpIter::Multi {
            sections,
            section_idx: 0,
            seg_idx: 0,
            header_emitted: false,
            blank_pending: false,
        }
    }
}

fn segment_to_line(seg: &HelpSegment) -> HelpLine {
    match seg {
        HelpSegment::Static(s) => HelpLine::Static(s),
        HelpSegment::OneOf { prefix, items } => HelpLine::Owned(render_one_of(prefix, items)),
    }
}

impl Iterator for HelpIter {
    type Item = HelpLine;
    fn next(&mut self) -> Option<HelpLine> {
        match self {
            HelpIter::Single { segments, idx } => {
                if *idx < segments.len() {
                    let line = segment_to_line(&segments[*idx]);
                    *idx += 1;
                    Some(line)
                } else {
                    None
                }
            }
            HelpIter::Multi {
                sections,
                section_idx,
                seg_idx,
                header_emitted,
                blank_pending,
            } => loop {
                if *section_idx >= sections.len() {
                    return None;
                }
                if *blank_pending {
                    *blank_pending = false;
                    return Some(HelpLine::Static(""));
                }
                let (header, segments) = sections[*section_idx];
                if !*header_emitted && !header.is_empty() {
                    *header_emitted = true;
                    return Some(HelpLine::Static(header));
                }
                if *seg_idx < segments.len() {
                    let line = segment_to_line(&segments[*seg_idx]);
                    *seg_idx += 1;
                    return Some(line);
                }
                // Section finished — advance and queue a blank line if more sections follow.
                *section_idx += 1;
                *seg_idx = 0;
                *header_emitted = false;
                if *section_idx < sections.len() {
                    *blank_pending = true;
                }
            },
        }
    }
}

/// Errors that can occur during runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeError {
    AllocFailed,
    BufferTooSmall,
    ColumnMismatch,
    Error,
    FlushFailed,
    HistoryQueueError,
    Msg(&'static str),
    ReadFailed,
    WriteFailed,
}

#[cfg(not(feature = "async-no-std"))]
use core::write;

#[cfg(not(feature = "async-no-std"))]
impl core::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RuntimeError::AllocFailed => write!(f, "Allocation failed"),
            RuntimeError::BufferTooSmall => write!(f, "Buffer too small"),
            RuntimeError::ColumnMismatch => write!(f, "Column mismatch"),
            RuntimeError::Error => write!(f, "Runtime error"),
            RuntimeError::FlushFailed => write!(f, "Flush failed"),
            RuntimeError::HistoryQueueError => write!(f, "Enqueue history error"),
            RuntimeError::Msg(msg) => write!(f, "{}", msg),
            RuntimeError::ReadFailed => write!(f, "Read failed"),
            RuntimeError::WriteFailed => write!(f, "Write failed"),
        }
    }
}

impl From<core::fmt::Error> for RuntimeError {
    fn from(_: core::fmt::Error) -> Self {
        RuntimeError::Error
    }
}

#[cfg(feature = "async-no-std")]
use defmt::{Format, Formatter, write};

#[cfg(feature = "async-no-std")]
impl Format for RuntimeError {
    fn format(&self, f: Formatter) {
        match self {
            RuntimeError::AllocFailed => write!(f, "Allocation failed"),
            RuntimeError::BufferTooSmall => write!(f, "Buffer too small"),
            RuntimeError::ColumnMismatch => write!(f, "Column mismatch"),
            RuntimeError::Error => write!(f, "Runtime error"),
            RuntimeError::FlushFailed => write!(f, "Flush failed"),
            RuntimeError::HistoryQueueError => write!(f, "Enqueue history error"),
            RuntimeError::Msg(msg) => write!(f, "{}", msg),
            RuntimeError::ReadFailed => write!(f, "Write failed"),
            RuntimeError::WriteFailed => write!(f, "Write failed"),
        }
    }
}
