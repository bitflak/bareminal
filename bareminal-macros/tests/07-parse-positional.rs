// Exercises positional argument support for struct-style variants.
//
// Positional rule: tokens that don't look like a flag are assigned to the
// next un-set, non-`Option<bool>` field in declaration order. Flags and
// positionals can interleave freely. `Option<bool>` flags stay
// presence-only and are never filled positionally.

use core::str::FromStr;

use bareminal_cli::{
    buffer::Buffer,
    process::{CommandsParser, ProcessError},
};
use bareminal_macros::Command;

#[derive(Debug, PartialEq, Command)]
enum Commands {
    /// Three positionals and a presence-only bool flag.
    GetAdc {
        value: i32,
        value2: i32,
        value3: Option<i32>,
        #[set(short = 'j')]
        json: Option<bool>,
    },
}

fn parse(s: &str) -> Result<Commands, ProcessError<'_>> {
    let buf = Buffer::<256>::from_str(s).unwrap();
    let buf: &'static mut Buffer<256> = Box::leak(Box::new(buf));
    let tokens = buf.as_tokens().unwrap();
    Commands::parse(&mut tokens.iter())
}

fn main() {
    // 1. All positional.
    assert_eq!(
        parse("get-adc 1 2 3"),
        Ok(Commands::GetAdc {
            value: 1,
            value2: 2,
            value3: Some(3),
            json: None,
        })
    );

    // 2. Flag then positional. Trailing positionals fill remaining fields
    //    in declaration order, skipping the one already set.
    assert_eq!(
        parse("get-adc --value2 42 100 200"),
        Ok(Commands::GetAdc {
            value: 100,
            value2: 42,
            value3: Some(200),
            json: None,
        })
    );

    // 3. Positional, then flag, then positional — full interleaving.
    assert_eq!(
        parse("get-adc 100 --value2 42 200"),
        Ok(Commands::GetAdc {
            value: 100,
            value2: 42,
            value3: Some(200),
            json: None,
        })
    );

    // 4. Bool flag mixed in at the end.
    assert_eq!(
        parse("get-adc 100 200 --json"),
        Ok(Commands::GetAdc {
            value: 100,
            value2: 200,
            value3: None,
            json: Some(true),
        })
    );

    // 5. Bool flag mixed in the middle — must not steal the following
    //    positional value.
    assert_eq!(
        parse("get-adc 100 --json 200 300"),
        Ok(Commands::GetAdc {
            value: 100,
            value2: 200,
            value3: Some(300),
            json: Some(true),
        })
    );

    // 6. Short bool flag.
    assert_eq!(
        parse("get-adc 100 200 -j"),
        Ok(Commands::GetAdc {
            value: 100,
            value2: 200,
            value3: None,
            json: Some(true),
        })
    );

    // 7. Optional positional left unset.
    assert_eq!(
        parse("get-adc 100 200"),
        Ok(Commands::GetAdc {
            value: 100,
            value2: 200,
            value3: None,
            json: None,
        })
    );

    // 8. Required positional missing — still surfaces MissingFlag.
    assert_eq!(parse("get-adc 100"), Err(ProcessError::MissingFlag("--value2")));

    // 9. Negative number is positional, not a flag.
    assert_eq!(
        parse("get-adc -5 -10"),
        Ok(Commands::GetAdc {
            value: -5,
            value2: -10,
            value3: None,
            json: None,
        })
    );
}
