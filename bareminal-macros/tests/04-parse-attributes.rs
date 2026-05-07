use core::str::FromStr;

use bareminal_cli::{
    buffer::Buffer,
    process::{CommandsParser, ProcessError},
};
use bareminal_macros::Command;

#[derive(Debug, PartialEq, Command)]
enum Commands {
    #[set(default = true)]
    Boolean(bool),
    #[set(default = 12)]
    TypeI32(i32),
    #[set(min = 2, max = 12)]
    TypeU32(u32),
    #[set(min = -2.0, max = 2.0)]
    TypeFloat(f32),
    #[set(one_of = [1,2])]
    TypeUsize(usize),
    #[set(one_of = ['a','b'])]
    TypeChar(char),
    #[set(min = 'b', max = 'c')]
    TypeChar2(char),
    #[set(default = (42,42), min = (42,42), max = (43,43))]
    TypeCompound((i32, i32)),
    Struct {
        #[set(default = Some(false), short)]
        some_flag: Option<bool>,
    },
    CustomShort {
        #[set(default = Some(false), short = 'c')]
        some_flag: Option<bool>,
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

fn assert_command_error(str: &str, error: ProcessError) {
    if let Ok(mut buffer) = Buffer::<256>::from_str(str)
        && let Some(tokens) = buffer.as_tokens()
    {
        let err = Commands::parse(&mut tokens.iter());
        assert_eq!(err, Err(error));
    }
}

fn main() {
    assert_command("boolean", Commands::Boolean(true));
    assert_command("type-i32", Commands::TypeI32(12));
    assert_command_error(
        "type-u32 42",
        ProcessError::OutOfRange(("type-u32", "u32", "2", "12")),
    );
    assert_command_error(
        "type-u32 1",
        ProcessError::OutOfRange(("type-u32", "u32", "2", "12")),
    );
    assert_command_error(
        "type-float -3.0",
        ProcessError::OutOfRange(("type-float", "f32", "-2.0", "2.0")),
    );
    assert_command_error(
        "type-float 3.0",
        ProcessError::OutOfRange(("type-float", "f32", "-2.0", "2.0")),
    );
    let arr: [&str; 2] = ["1", "2"];
    assert_command_error(
        "type-usize 42",
        ProcessError::NotInSet(("type-usize", "usize", &arr)),
    );
    let arr: [&str; 2] = ["a", "b"];
    assert_command_error(
        "type-char x",
        ProcessError::NotInSet(("type-char", "char", &arr)),
    );
    assert_command_error(
        "type-char2 a",
        ProcessError::OutOfRange(("type-char2", "char", "'b'", "'c'")),
    );
    assert_command_error(
        "type-char2 d",
        ProcessError::OutOfRange(("type-char2", "char", "'b'", "'c'")),
    );
    assert_command("type-compound", Commands::TypeCompound((42, 42)));
    assert_command_error(
        "type-compound 41 41",
        ProcessError::OutOfRange(("type-compound", "(i32, i32)", "(42, 42)", "(43, 43)")),
    );
    assert_command_error(
        "type-compound 44 44",
        ProcessError::OutOfRange(("type-compound", "(i32, i32)", "(42, 42)", "(43, 43)")),
    );
    assert_command(
        "struct",
        Commands::Struct {
            some_flag: Some(false),
        },
    );
    assert_command(
        "struct --some-flag",
        Commands::Struct {
            some_flag: Some(true),
        },
    );
    assert_command(
        "struct -s",
        Commands::Struct {
            some_flag: Some(true),
        },
    );
    assert_command(
        "custom-short -c",
        Commands::CustomShort {
            some_flag: Some(true),
        },
    );
}
