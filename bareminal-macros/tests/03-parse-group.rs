use core::str::FromStr;

use bareminal_cli::{buffer::Buffer, process::CommandsParser};
use bareminal_macros::{Command, CommandGroup};

#[derive(PartialEq, Debug, Command)]
enum BaseCommands<'a> {
    Simple(&'a str),
}

#[derive(PartialEq, Debug, CommandGroup)]
enum CommandGroup<'a> {
    Base(BaseCommands<'a>),
}

fn assert_command(str: &str, command: CommandGroup<'_>) {
    if let Ok(mut buffer) = Buffer::<256>::from_str(str)
        && let Some(tokens) = buffer.as_tokens()
    {
        let cmd = CommandGroup::parse(&mut tokens.iter());
        assert_eq!(cmd, Ok(command));
    }
}

fn main() {
    assert_command(
        "simple hello",
        CommandGroup::Base(BaseCommands::Simple("hello")),
    );
}
