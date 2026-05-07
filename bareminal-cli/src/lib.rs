#![doc = include_str!("../../README.md")]
#![cfg_attr(not(any(feature = "std", feature = "async-std")), no_std)]
#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod buffer;
pub mod bytes;
pub mod cmdline;
pub mod codes;
pub mod input;
pub mod process;
pub mod tokens;
pub mod utf;

#[cfg(any(feature = "async-no-std", feature = "async-std"))]
mod cli_async;

#[cfg(feature = "std")]
mod cli_sync;

pub mod cli {
    use super::*;

    #[cfg(all(
        any(feature = "async-no-std", feature = "async-std"),
        not(feature = "std")
    ))]
    pub use cli_async::*;

    #[cfg(feature = "std")]
    pub use cli_sync::*;
}
