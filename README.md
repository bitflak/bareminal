# bareminal

**bareminal** is a command line interface inspired by [embedded-cli](https://github.com/funbiscuit/embedded-cli-rs). It runs asynchronously in `no_std` environments, and either synchronously or asynchronously in `std` environments. A synchronous version for `no_std` is not yet supported — if you need that, check out [embedded-cli](https://github.com/funbiscuit/embedded-cli-rs).

Dual-licensed under [Apache 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT).

## Features

- [x] Command declaration with enums
- [x] Clear line support (ctrl+u)
- [x] Command autocompletion (tab)
- [x] Configurable memory usage
- [x] Command editing (left/right arrows)
- [x] Default command values
- [x] Help generated from doc comments
- [x] History (up/down arrows)
- [x] No dynamic dispatch
- [x] Optional values and flags support
- [x] Panic-free
- [x] Partial color support for errors
- [x] Primitive and complex argument parsing
- [x] Render in a loop (async only)
- [x] Render lists, tables, and JSON
- [x] Static allocation
- [x] Subcommand support
- [x] UTF-8 support
- [x] Value validation (min, max, one_of)

## Usage

Add bareminal dependency:

```toml
bareminal_macros = { version = "0.1" }

bareminal_cli = { version = "0.1", default-features = false, features = [
  "async-no-std",
] }

// or
bareminal_cli = { version = "0.1", default-features = false, features = [
  "async-std",
] }


// or
bareminal_cli = { version = "0.1", default-features = false, features = [
  "std",
] }
```

Declare your commands by deriving the `Command` proc macro for an enum:

```rust,ignore
/// General commands description.
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
        /// you can define a custome version
        #[set(default = "is awesome", short = 'm')]
        name2: &'a str,
        /// Snake case is again transformed to kebab-case.
        /// On boolean flags you can omit the value so
        /// just --some-flag is same as --some-flag true
        #[set(default = Some(false))]
        some_flag: Option<bool>,
    },
    /// Async version also provides a special loop method.
    /// See the documentation for more information and see examples
    Loop,
}
```

You can also group your commands by deriving the `CommandGroup` proc macro if
you want to split your commands into individual modules:

```rust,ignore
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
```

Processing of the input happens on byte level and depends on the environment
you chose. You can look at examples for more details, but here are few examples:

### Async no_std

```rust,ignore

#[derive(Debug, Command)]
enum BaseCommands {
  Simple,
}

#[derive(Debug, CommandGroup)]
enum CommandGroup {
    Base(BaseCommands),
}

const MAX_CMD_BUFFER: usize = 256;
const HISTORY_SIZE: usize = 3;

let (tx, rx) = uart.split();

let mut cli =
    match Bareminal::<CommandGroup, _, MAX_CMD_BUFFER, HISTORY_SIZE>::new(tx).await {
        Ok(cli) => cli,
        Err(err) => {
            error!("{}", err);
            return;
        }
    };

let buf = unsafe {
    static mut BUF: [u8; 256] = [0u8; 256];
    &mut *core::ptr::addr_of_mut!(BUF)
};

let mut rx = rx.into_ring_buffered(buf);

let mut read_buf = [0u8; 64];

loop {
    match rx.read(&mut read_buf[..]).await {
        Ok(received) => {
            'outer: for &byte in &read_buf[..received] {
                if let Ok(ready) = cli
                    .add_byte(byte)
                    .await
                    .inspect_err(|err| error!("{}", err))
                    && ready
                {
                    loop {
                        let result = cli.next_command().await;
                        match result {
                            Ok(None) => {
                                if let Err(err) = cli.finalize().await {
                                    error!("{}", err);
                                    break 'outer;

                                }
                                break;
                            }
                            Ok(Some((command, writer))) => {
                                process_command(command, writer).await;
                            }
                            Err(err) => {
                                error!("{}", err);
                                break 'outer;
                            }
                        };
                    }
                }
            }
        }
        Err(err) => {
            error!("{}", err);
            break;
        }
    }
}

async fn process_command<
    'a,
    W: embedded_io_async::Write + Unpin,
    R: embedded_io_async::Read + Unpin + ?Sized,
>(
    command: CommandGroup,
    writer: &'a mut CommandWriter<W>,
) {
    match command {
        CommandGroup::Base(base_command) => match base_command {
            BaseCommands::Simple => {
                let _ = writer
                    .write_line("simple command".as_bytes())
                    .await
                    .inspect_err(|err| error!("{}", err));
            }
        },
    };
}
```

### Async std

```rust,ignore
use tokio::io;

#[derive(Debug, Command)]
enum BaseCommands {
  Simple,
}

#[derive(Debug, CommandGroup)]
enum CommandGroup {
    Base(BaseCommands),
}

const MAX_CMD_BUFFER: usize = 256;
const HISTORY_SIZE: usize = 3;

let stdin = io::stdin();
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
                            BaseCommands::Simple => {
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
```

### Sync std

```rust,ignore

#[derive(Debug, Command)]
enum BaseCommands {
  Simple,
}

#[derive(Debug, CommandGroup)]
enum CommandGroup {
    Base(BaseCommands),
}

const MAX_CMD_BUFFER: usize = 256;
const HISTORY_SIZE: usize = 3;


let stdin = io::stdin();
let mut stdin = stdin.lock();
let mut buf = [0u8; 4];

let mut cli = Bareminal::<CommandGroup, _, MAX_COMMAND_BUFFER, HISTORY_SIZE>::new(io::stdout())
    .map_err(|_| anyhow::Error::msg("CLI initialization failed"))?;

loop {
    let read = stdin.read(&mut buf)?;

    if read == 0 {
        break;
    }

    if buf[0] == 0x03 {
        break;
    }

    for &byte in &buf[..read] {
        let _ = cli
            .add_byte(byte, |command, writer| match command {
                CommandGroup::Base(base_command) => match base_command {
                    BaseCommands::Simple => {
                        let _ = writer.write_line("Simple command".as_bytes());
                    }
                },
            })
            .inspect_err(|err| eprintln!("{}", err));
    }
}
```

### Direct command parsing

You can also parse commands directly like so:

```rust,ignore
#[derive(Debug, PartialEq, Command)]
enum Commands<'a> {
    Boolean(bool),
    Str(&'a str),
    Complex(heapless::String<128>),
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
    assert_command("str hello", Commands::Str("hello"));
    let str = heapless::String::<128>::try_from("hello").unwrap();
    assert_command("complex hello", Commands::Complex(str));
}
```

## Command usage

The `help` command would produce following output for above example:

```text
== BaseCommands ==
General command description.
Multiline also possible.

Commands:
  simple-command    Help for individual command
  int               You can set default and min and max values
  float             All primitive types are supported
  char              Min and max values are also possible for chars
  set               You can define a set of possible values by one_of attribute
  compund           Compound values are possible too
  list              You can print optionally enumerated lists
  table             You can print tables
  some-json         You can print or pretty print json values directly
  modes             Enum values are possible too, but you have to
  command           Command can also have flags.
  loop              Async version also provides a special loop method.

// ... other commands
```

And `help <command name>` would produce:

```text
== command ==
Command can also have flags.
Use struct like enum variants for this
command --name <string> --name2 <string> [--some-flag]

Flags:
  --name, -n          Same attributes are possible
                      additionally you can set short attribute
                      if set, short flag version -n will be possible
                      [default: Bareminal]
  --name2, -m         If short version collides with something else
                      you can define a custom version
                      [default: is awesome]
  [--some-flag]       Snake case is again transformed to kebab-case.
                      On boolean flags you can omit the value so
                      just --some-flag is same as --some-flag true
                      [default: false]
```

Note: appending `--help` to a command, does not work for now.

Taking the example from above you can write any command and combine the flags in any order:

```bash
command --some-flag --name hello
```

You can wrap the input into double quotes if you want to input a space separated string

```bash
command --some-flag --name "hello world\'s"
```

You can also chain command:

```bash
int 42 char b
```

Because commands can also have a default value, it can lead to some ambiguity
like in this case, where `char` is not the input of int:

```bash
int char b
```

To disambiguate the input, you can separate individual commands by using `--` like so:

```bash
int -- char b -- simple
```

## Credits

- [embedded-cli](https://github.com/funbiscuit/embedded-cli-rs)
- [ANSI Escape Sequences](https://gist.github.com/ConnerWill/d4b6c776b509add763e17f9f113fd25b)
