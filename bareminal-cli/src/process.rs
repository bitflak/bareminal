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
    fn help() -> &'static [&'static str];
    fn help_for(name: &str) -> &'static [&'static str];
    fn help_lines() -> HelpIter {
        HelpIter::single(Self::help())
    }
}

pub enum HelpIter {
    Single {
        lines: &'static [&'static str],
        idx: usize,
    },
    Multi {
        sections: &'static [(&'static str, &'static [&'static str])],
        section_idx: usize,
        line_idx: usize,
        header_emitted: bool,
        blank_pending: bool,
    },
}

impl HelpIter {
    pub const fn single(lines: &'static [&'static str]) -> Self {
        HelpIter::Single { lines, idx: 0 }
    }

    pub const fn multi(sections: &'static [(&'static str, &'static [&'static str])]) -> Self {
        HelpIter::Multi {
            sections,
            section_idx: 0,
            line_idx: 0,
            header_emitted: false,
            blank_pending: false,
        }
    }
}

impl Iterator for HelpIter {
    type Item = &'static str;
    fn next(&mut self) -> Option<&'static str> {
        match self {
            HelpIter::Single { lines, idx } => {
                if *idx < lines.len() {
                    let line = lines[*idx];
                    *idx += 1;
                    Some(line)
                } else {
                    None
                }
            }
            HelpIter::Multi {
                sections,
                section_idx,
                line_idx,
                header_emitted,
                blank_pending,
            } => loop {
                if *section_idx >= sections.len() {
                    return None;
                }
                if *blank_pending {
                    *blank_pending = false;
                    return Some("");
                }
                let (header, lines) = sections[*section_idx];
                if !*header_emitted && !header.is_empty() {
                    *header_emitted = true;
                    return Some(header);
                }
                if *line_idx < lines.len() {
                    let line = lines[*line_idx];
                    *line_idx += 1;
                    return Some(line);
                }
                // Section finished — advance and queue a blank line if more sections follow.
                *section_idx += 1;
                *line_idx = 0;
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
