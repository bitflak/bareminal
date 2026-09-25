use bareminal_cli::process::CommandsParser;
use bareminal_macros::{Command, CommandGroup};

#[derive(Debug, Command)]
enum CmdA {
    SimpleA,
}

#[derive(Debug, Command)]
enum CmdB {
    SimpleB,
}

#[derive(Debug, CommandGroup)]
enum NestedGroup {
    B(CmdB),
}

#[derive(Debug, CommandGroup)]
enum MainGroup {
    A(CmdA),
    Nested(NestedGroup),
}

fn main() {
    let lines: Vec<String> = MainGroup::help_lines()
        .map(|l| l.as_ref().to_string())
        .collect();

    // Commands from a directly nested group must appear in the group help.
    assert!(
        lines.iter().any(|l| l.contains("simple-a")),
        "expected simple-a in help, got: {:?}",
        lines
    );
    assert!(
        lines.iter().any(|l| l.contains("simple-b")),
        "expected simple-b (from nested CommandGroup) in help, got: {:?}",
        lines
    );
    assert!(
        lines.iter().any(|l| l == "== CmdA =="),
        "expected CmdA section header, got: {:?}",
        lines
    );
    assert!(
        lines.iter().any(|l| l == "== CmdB =="),
        "expected CmdB section header from nested group, got: {:?}",
        lines
    );
}
