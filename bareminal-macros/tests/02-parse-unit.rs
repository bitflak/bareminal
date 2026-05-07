use core::str::FromStr;

use bareminal_cli::{buffer::Buffer, process::CommandsParser};
use bareminal_macros::Command;

#[derive(Debug, PartialEq)]
enum Modes {
    Mode1,
    Mode2,
}

impl FromStr for Modes {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mode1" => Ok(Modes::Mode1),
            "mode2" => Ok(Modes::Mode2),
            _ => Err(()),
        }
    }
}

#[derive(Debug, PartialEq, Command)]
enum Commands<'a> {
    Boolean(bool),
    TypeI32(i32),
    TypeU32(u32),
    TypeUsize(usize),
    Str(&'a str),
    Complex(heapless::String<128>),
    Modes(Modes),
    Struct { name: &'a str },
}

fn assert_command(str: &str, command: Commands<'_>) {
    if let Ok(mut buffer) = Buffer::<256>::from_str(str)
        && let Some(tokens) = buffer.as_tokens()
    {
        let cmd = Commands::parse(&mut tokens.iter());
        assert_eq!(cmd, Ok(command));
    }
}

fn main() {
    assert_command("boolean true", Commands::Boolean(true));
    assert_command("type-i32 42", Commands::TypeI32(42));
    assert_command("type-u32 42", Commands::TypeU32(42));
    assert_command("type-usize 42", Commands::TypeUsize(42));
    assert_command("str hello", Commands::Str("hello"));
    let str = heapless::String::<128>::try_from("hello").unwrap();
    assert_command("complex hello", Commands::Complex(str));
    assert_command("modes mode1", Commands::Modes(Modes::Mode1));
    assert_command("struct --name cat", Commands::Struct { name: "cat" });
}
