#[test]
fn tests() {
    let t = trybuild::TestCases::new();
    t.pass("tests/01-parse-simple.rs");
    t.pass("tests/02-parse-unit.rs");
    t.pass("tests/03-parse-group.rs");
    t.pass("tests/04-parse-attributes.rs");
    t.pass("tests/05-parse-flags.rs");
}
