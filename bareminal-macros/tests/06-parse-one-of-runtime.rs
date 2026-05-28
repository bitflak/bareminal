// Exercises `one_of` with a non-array expression (a `const &[&str]`).
//
// The macro should:
//   1. Accept the expression at parse time.
//   2. Use the slice for the runtime membership check.
//   3. Pass it straight through to `NotInSet` so the error reports the
//      actual allowed values.
//   4. Surface the runtime values in `help_for` via a `HelpSegment::OneOf`.

extern crate alloc;

use alloc::string::ToString;
use alloc::vec::Vec;
use core::str::FromStr;

use bareminal_cli::{
    buffer::Buffer,
    process::{CommandsParser, HelpIter, HelpSegment, ProcessError},
};
use bareminal_macros::Command;

mod adc {
    pub const VARIANTS: &[&str] = &["t_die", "t_foo"];
}

#[derive(Debug, PartialEq)]
enum AdcSignal {
    TDie,
    TFoo,
}

impl FromStr for AdcSignal {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "t_die" => Ok(Self::TDie),
            "t_foo" => Ok(Self::TFoo),
            _ => Err(()),
        }
    }
}

#[derive(Debug, PartialEq, Command)]
enum Commands {
    /// Prints ADC signal value
    GetAdc {
        /// Adc signal type (see list-adc)
        #[set(short, one_of = adc::VARIANTS)]
        value: AdcSignal,
    },
}

fn main() {
    // Valid value passes membership check.
    let mut buf = Buffer::<256>::from_str("get-adc --value t_die").unwrap();
    let tokens = buf.as_tokens().unwrap();
    let cmd = Commands::parse(&mut tokens.iter()).unwrap();
    assert_eq!(
        cmd,
        Commands::GetAdc {
            value: AdcSignal::TDie,
        }
    );

    // Disallowed value should report `NotInSet` carrying the runtime slice.
    let mut buf = Buffer::<256>::from_str("get-adc --value bogus").unwrap();
    let tokens = buf.as_tokens().unwrap();
    let err = Commands::parse(&mut tokens.iter()).unwrap_err();
    match err {
        ProcessError::NotInSet((ctx, _ty, allowed)) => {
            assert_eq!(ctx, "get-adc.value");
            // The slice must be the same `const &[&str]` we passed in,
            // not a synthesized list — verify by value.
            assert_eq!(allowed, adc::VARIANTS);
        }
        other => panic!("expected NotInSet, got {:?}", other),
    }

    // Help for `get-adc` should carry a HelpSegment::OneOf pointing at
    // the runtime slice (not pre-baked into static strings).
    let help = Commands::help_for("get-adc");
    let has_runtime_segment = help
        .iter()
        .any(|seg| matches!(seg, HelpSegment::OneOf { items, .. } if *items == adc::VARIANTS));
    assert!(
        has_runtime_segment,
        "expected a HelpSegment::OneOf carrying adc::VARIANTS in help_for output"
    );

    // And the iterator should expand it into a line containing the values.
    let rendered: Vec<_> = HelpIter::single(help)
        .map(|line| line.as_ref().to_string())
        .collect();
    assert!(
        rendered.iter().any(|l| l.contains("[one of: t_die, t_foo]")),
        "expected runtime one_of values in rendered help, got: {:?}",
        rendered
    );
}
