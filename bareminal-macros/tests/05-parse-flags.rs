use core::str::FromStr;

use bareminal_cli::{buffer::Buffer, process::CommandsParser};
use bareminal_macros::Command;

#[derive(Debug, PartialEq, Command)]
enum Commands<'a> {
    Command {
        str: &'a str,
        num: i32,
        comp: (i32, i32),
        boolean: bool,
    },
}

fn assert_command(str: &str, command: Commands) {
    if let Ok(mut buffer) = Buffer::<256>::from_str(str)
        && let Some(tokens) = buffer.as_tokens()
    {
        let cmd = Commands::parse(&mut tokens.iter());
        assert_eq!(cmd, Ok(command));
    }
}

fn main() {
    assert_command(
        "command --str cat --num -3 --comp -1 1 --boolean false",
        Commands::Command {
            str: "cat",
            num: -3,
            comp: (-1, 1),
            boolean: false,
        },
    );
}
