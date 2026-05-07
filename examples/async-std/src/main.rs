use core::fmt::Write;
use std::os::fd::AsRawFd;

use heapless::String;
use rand::prelude::*;
use serde::{Deserialize, Serialize};
use tokio::io::{self, AsyncReadExt};

use bareminal_cli::cli::Bareminal;
use bareminal_macros::{Command, CommandGroup};

use core::fmt;

/// General command description.
/// Multiline also possible.
#[derive(Debug, Command)]
enum BaseCommands<'a> {
    /// Help for individual command
    /// In the help overview, only the first line is visible
    /// CamelCase name will be transformed into kebab-case
    SimpleCommand,
    /// You can set default and min and max values
    #[set(default = 2, min = -5, max = 3)]
    Int(i32),
    /// All primitive types are supported
    #[set(min = -1.0, max = 2.0)]
    Float(f32),
    /// Min and max values are also possible for chars
    #[set(default = 'a', max = 'c')]
    Char(char),
    /// You can define a set of possible values by one_of attribute
    #[set(default = 'a', one_of = ['a', 'b'])]
    Set(char),
    /// Compound values are possible too
    #[set(default = (42, 42), min = (1,1), max = (42,42))]
    Compound((i32, i32)),
    /// You can print optionally enumerated lists
    List,
    /// You can print tables
    Table,
    /// You can print or pretty print json values directly
    /// Uses serde and serde_json_core under the hood
    SomeJson {
        #[set(default = Some(false))]
        json: Option<bool>,
        #[set(default = Some(false))]
        pretty: Option<bool>,
    },
    /// Enum values are possible too, but you have to
    /// implement FromStr trait. A bit cumbersome, but
    /// it also gives you some flexibility.
    #[set(default = "mode1", one_of = ["mode1", "mode2"])]
    Modes(Option<Modes>),
    /// Command can also have flags.
    /// Use struct like enum variants for this
    Command {
        /// Same attributes are possible
        /// additionally you can set short attribute
        /// if set, short flag version -n will be possible
        #[set(default = "Bareminal", short)]
        name: &'a str,
        /// If short version collides with something else
        /// you can define a custom version
        #[set(default = "is awesome", short = 'm')]
        name2: &'a str,
        /// Snake case is again transformed to kebab-case.
        /// On boolean flags you can omit the value so
        /// just --some-flag is same as --some-flag true
        #[set(default = Some(false))]
        some_flag: Option<bool>,
    },
    /// Async version also provides a special loop method.
    /// See the documentation for more information
    Loop,
}

/// Some second command group
#[derive(Debug, Command)]
enum SecondCommandGroup {
    /// When two commands have a similar name,
    /// autocomplete will pick the shorter one.
    Simple2,
}

/// Doc comments on CommandGroup will be ignored
#[derive(Debug, CommandGroup)]
enum CommandGroup<'a> {
    Base(BaseCommands<'a>),
    Second(SecondCommandGroup),
}

#[derive(Serialize, Deserialize)]
struct Nested {
    meta: u8,
}

#[derive(Serialize, Deserialize)]
struct SomeJson {
    pin: u8,
    value: u8,
    nested: Nested,
}

impl fmt::Display for SomeJson {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(pin: {}, value: {})", self.pin, self.value)
    }
}

#[derive(Debug, PartialEq)]
enum Modes {
    Mode1,
    Mode2,
}

use core::str::FromStr;

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

use termios::{TCSAFLUSH, Termios, VMIN, VTIME, cfmakeraw, tcsetattr};

struct RawModeGuard {
    fd: i32,
    original: Termios,
}

impl RawModeGuard {
    fn enable(fd: i32) -> anyhow::Result<Self> {
        let original = Termios::from_fd(fd)?;

        let mut raw = original;
        cfmakeraw(&mut raw);
        raw.c_cc[VMIN] = 1; // read returns after at least 1 byte
        raw.c_cc[VTIME] = 0; // no inter-byte timeout
        tcsetattr(fd, TCSAFLUSH, &raw)?;

        Ok(Self { fd, original })
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = tcsetattr(self.fd, TCSAFLUSH, &self.original);
    }
}

const MAX_CMD_BUFFER: usize = 256;
const HISTORY_SIZE: usize = 3;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let stdin = tokio::io::stdin();
    let fd = stdin.as_raw_fd();

    let _guard = RawModeGuard::enable(fd)?;

    let mut stdin = io::stdin();
    let mut buf = [0u8; 4];

    let mut cli = Bareminal::<CommandGroup, _, MAX_CMD_BUFFER, HISTORY_SIZE>::new(io::stdout())
        .await
        .map_err(|_| anyhow::Error::msg("CLI initialization failed"))?;

    loop {
        let read = stdin.read(&mut buf).await?;

        if read == 0 {
            break;
        }

        if buf[0] == 0x03 {
            break;
        }

        let some_json = SomeJson {
            pin: 42,
            value: 42,
            nested: Nested { meta: 42 },
        };

        'outer: for &byte in &buf[..read] {
            if let Ok(ready) = cli
                .add_byte(byte)
                .await
                .inspect_err(|err| eprintln!("{}", err))
                && ready
            {
                loop {
                    let result = cli.next_command().await;
                    match result {
                        Ok(None) => {
                            if let Err(err) = cli.finalize().await {
                                eprintln!("{}", err);
                                break 'outer;
                            }
                            break;
                        }
                        Ok(Some((command, writer))) => match command {
                            CommandGroup::Base(base_command) => match base_command {
                                BaseCommands::SimpleCommand => {
                                    let _ = writer.write_line("Simple command".as_bytes()).await;
                                }
                                BaseCommands::Int(value) => {
                                    let _ =
                                        writer.write_line(format!("{:?}", value).as_bytes()).await;
                                }
                                BaseCommands::Float(value) => {
                                    let _ =
                                        writer.write_line(format!("{:?}", value).as_bytes()).await;
                                }
                                BaseCommands::Char(value) => {
                                    let _ =
                                        writer.write_line(format!("{:?}", value).as_bytes()).await;
                                }
                                BaseCommands::Set(value) => {
                                    let _ =
                                        writer.write_line(format!("{:?}", value).as_bytes()).await;
                                }
                                BaseCommands::Compound((value1, value2)) => {
                                    let _ = writer
                                        .write_line(
                                            format!("{:?}, {:?}", value1, value2).as_bytes(),
                                        )
                                        .await;
                                }
                                BaseCommands::List => {
                                    let rows: &[&[&str]] = &[&["Pin A", "30"], &["Pin B", "42"]];
                                    let _ = writer.write_list::<256>(rows, true).await;
                                }
                                BaseCommands::Table => {
                                    let headers = ["Name", "Value"];
                                    let rows: &[&[&str]] = &[&["Pin A", "30"], &["Pin B", "42"]];
                                    let _ = writer.write_table::<256>(&headers, rows).await;
                                }
                                BaseCommands::SomeJson { json, pretty } => {
                                    if let Some(true) = json {
                                        let _ = writer
                                            .write_json::<512>(&some_json, pretty.unwrap_or(false))
                                            .await;
                                    } else {
                                        let _ = writer
                                            .write_line(format!("{}", some_json).as_bytes())
                                            .await;
                                    };
                                }
                                BaseCommands::Command {
                                    name,
                                    name2,
                                    some_flag,
                                } => {
                                    let _ = writer
                                        .write_line(
                                            format!("{:?} {:?} {:?}", name, name2, some_flag)
                                                .as_bytes(),
                                        )
                                        .await;
                                }
                                BaseCommands::Modes(value) => {
                                    let _ =
                                        writer.write_line(format!("{:?}", value).as_bytes()).await;
                                }
                                BaseCommands::Loop => {
                                    let mut rng = rand::rng();
                                    let _ = writer
                                        .write_loop::<128>(
                                            &mut io::stdin(),
                                            async move |mut buf| {
                                                let r = rng.random_range(10000..=90000);
                                                let headers = ["Name", "Value"];
                                                let mut s: String<32> = String::new();
                                                let _ = write!(s, "{}", r);
                                                let rows: &[&[&str]] =
                                                    &[&["Pin A", "30"], &["Pin B", &s]];
                                                buf.write_table(&headers, rows).unwrap();
                                                buf
                                            },
                                        )
                                        .await;
                                }
                            },
                            CommandGroup::Second(command) => match command {
                                SecondCommandGroup::Simple2 => {
                                    let _ =
                                        writer.write_line("Second simple command".as_bytes()).await;
                                }
                            },
                        },
                        Err(err) => {
                            eprintln!("{}", err);
                            break 'outer;
                        }
                    }
                }
            }
        }
    }

    let _ = cli
        .write(b"\r\n")
        .await
        .inspect_err(|err| eprintln!("{}", err));

    Ok(())
}
