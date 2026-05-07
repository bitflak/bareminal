use crate::{codes, utf};
use core::default::Default;
use core::fmt::Debug;
use core::option::Option;
use core::option::Option::None;
use core::option::Option::Some;
use core::prelude::rust_2024::derive;

#[derive(Debug)]
pub enum Control {
    Backspace,
    Down,
    Enter,
    Left,
    Right,
    Tab,
    Up,
    Formfeed,
    Exit,
}

#[derive(Debug)]
pub enum InputType<'a> {
    Ctrl(Control),
    Char(&'a str),
}

/// Handling different decoding types
#[derive(Default)]
pub struct InputParser {
    acc: utf::Utf8Accumulator,
    csi: bool,
    prev_byte: u8,
}

impl InputParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_byte(&mut self, byte: u8) -> Option<InputType<'_>> {
        let prev_byte = self.prev_byte;
        self.prev_byte = byte;
        if self.csi {
            self.csi = false;
            let control = match byte {
                b'A' => Control::Up,
                b'B' => Control::Down,
                b'C' => Control::Right,
                b'D' => Control::Left,
                _ => return None,
            };
            Some(InputType::Ctrl(control))
        } else if prev_byte == crate::codes::ESCAPE && byte == b'[' {
            self.csi = true;
            None
        } else {
            let input = match byte {
                codes::BACKSPACE | codes::DEL => Control::Backspace,
                codes::EXIT => Control::Exit,
                codes::FORM_FEED => Control::Formfeed,
                codes::CARRIAGE_RETURN if prev_byte != codes::LINE_FEED => Control::Enter,
                codes::LINE_FEED if prev_byte != codes::CARRIAGE_RETURN => Control::Enter,
                codes::TAB => Control::Tab,
                byte if byte >= 0x20 => return self.acc.push(byte).map(InputType::Char),
                _ => return None,
            };
            Some(InputType::Ctrl(input))
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    enum Owned {
        Ctrl(ControlKind),
        Char(String),
    }

    #[derive(Debug, PartialEq, Eq)]
    enum ControlKind {
        Backspace,
        Down,
        Enter,
        Left,
        Right,
        Tab,
        Up,
        Formfeed,
        Exit,
    }

    fn kind_of(c: &Control) -> ControlKind {
        match c {
            Control::Backspace => ControlKind::Backspace,
            Control::Down => ControlKind::Down,
            Control::Enter => ControlKind::Enter,
            Control::Left => ControlKind::Left,
            Control::Right => ControlKind::Right,
            Control::Tab => ControlKind::Tab,
            Control::Up => ControlKind::Up,
            Control::Formfeed => ControlKind::Formfeed,
            Control::Exit => ControlKind::Exit,
        }
    }

    fn feed(parser: &mut InputParser, bytes: &[u8]) -> Vec<Owned> {
        let mut out = Vec::new();
        for &b in bytes {
            if let Some(event) = parser.push_byte(b) {
                let owned = match event {
                    InputType::Ctrl(c) => Owned::Ctrl(kind_of(&c)),
                    InputType::Char(s) => Owned::Char(s.to_string()),
                };
                out.push(owned);
            }
        }
        out
    }

    #[test]
    fn backspace_byte_emits_backspace() {
        let mut p = InputParser::new();
        assert_eq!(
            feed(&mut p, &[0x08]),
            vec![Owned::Ctrl(ControlKind::Backspace)]
        );
    }

    #[test]
    fn del_byte_also_emits_backspace() {
        let mut p = InputParser::new();
        assert_eq!(
            feed(&mut p, &[0x7F]),
            vec![Owned::Ctrl(ControlKind::Backspace)]
        );
    }

    #[test]
    fn tab_emits_tab() {
        let mut p = InputParser::new();
        assert_eq!(feed(&mut p, &[0x09]), vec![Owned::Ctrl(ControlKind::Tab)]);
    }

    #[test]
    fn formfeed_emits_formfeed() {
        let mut p = InputParser::new();
        assert_eq!(
            feed(&mut p, &[0x0C]),
            vec![Owned::Ctrl(ControlKind::Formfeed)]
        );
    }

    #[test]
    fn exit_byte_emits_exit() {
        // Assumes codes::EXIT == 0x03; change if different.
        let mut p = InputParser::new();
        assert_eq!(feed(&mut p, &[0x03]), vec![Owned::Ctrl(ControlKind::Exit)]);
    }

    // ---------- Enter / CRLF handling ----------

    #[test]
    fn lone_lf_emits_enter() {
        let mut p = InputParser::new();
        assert_eq!(feed(&mut p, b"\n"), vec![Owned::Ctrl(ControlKind::Enter)]);
    }

    #[test]
    fn lone_cr_emits_enter() {
        let mut p = InputParser::new();
        assert_eq!(feed(&mut p, b"\r"), vec![Owned::Ctrl(ControlKind::Enter)]);
    }

    #[test]
    fn crlf_emits_single_enter() {
        // \r should produce Enter, then \n should be suppressed because prev_byte == \r.
        let mut p = InputParser::new();
        assert_eq!(feed(&mut p, b"\r\n"), vec![Owned::Ctrl(ControlKind::Enter)]);
    }

    #[test]
    fn lfcr_emits_single_enter() {
        // The reverse: \n then \r. Symmetrically, the second is suppressed.
        let mut p = InputParser::new();
        assert_eq!(feed(&mut p, b"\n\r"), vec![Owned::Ctrl(ControlKind::Enter)]);
    }

    #[test]
    fn two_separate_lfs_emit_two_enters() {
        let mut p = InputParser::new();
        assert_eq!(
            feed(&mut p, b"\n\n"),
            vec![
                Owned::Ctrl(ControlKind::Enter),
                Owned::Ctrl(ControlKind::Enter)
            ],
        );
    }

    #[test]
    fn two_separate_crs_emit_two_enters() {
        let mut p = InputParser::new();
        assert_eq!(
            feed(&mut p, b"\r\r"),
            vec![
                Owned::Ctrl(ControlKind::Enter),
                Owned::Ctrl(ControlKind::Enter)
            ],
        );
    }

    #[test]
    fn crlf_then_text_then_crlf() {
        let mut p = InputParser::new();
        let out = feed(&mut p, b"\r\nhi\r\n");
        assert_eq!(
            out,
            vec![
                Owned::Ctrl(ControlKind::Enter),
                Owned::Char("h".into()),
                Owned::Char("i".into()),
                Owned::Ctrl(ControlKind::Enter),
            ],
        );
    }

    #[test]
    fn arrow_up() {
        let mut p = InputParser::new();
        assert_eq!(
            feed(&mut p, &[0x1B, b'[', b'A']),
            vec![Owned::Ctrl(ControlKind::Up)]
        );
    }

    #[test]
    fn arrow_down() {
        let mut p = InputParser::new();
        assert_eq!(
            feed(&mut p, &[0x1B, b'[', b'B']),
            vec![Owned::Ctrl(ControlKind::Down)]
        );
    }

    #[test]
    fn arrow_right() {
        let mut p = InputParser::new();
        assert_eq!(
            feed(&mut p, &[0x1B, b'[', b'C']),
            vec![Owned::Ctrl(ControlKind::Right)]
        );
    }

    #[test]
    fn arrow_left() {
        let mut p = InputParser::new();
        assert_eq!(
            feed(&mut p, &[0x1B, b'[', b'D']),
            vec![Owned::Ctrl(ControlKind::Left)]
        );
    }

    #[test]
    fn esc_alone_emits_nothing() {
        let mut p = InputParser::new();
        assert_eq!(feed(&mut p, &[0x1B]), vec![]);
    }

    #[test]
    fn esc_followed_by_bracket_emits_nothing_yet() {
        let mut p = InputParser::new();
        assert_eq!(feed(&mut p, &[0x1B, b'[']), vec![]);
    }

    #[test]
    fn unknown_csi_final_byte_is_swallowed() {
        let mut p = InputParser::new();
        let out = feed(&mut p, &[0x1B, b'[', b'Z', b'x']);
        assert_eq!(out, vec![Owned::Char("x".into())]);
    }

    #[test]
    fn arrow_followed_by_text() {
        let mut p = InputParser::new();
        let out = feed(&mut p, &[0x1B, b'[', b'A', b'h', b'i']);
        assert_eq!(
            out,
            vec![
                Owned::Ctrl(ControlKind::Up),
                Owned::Char("h".into()),
                Owned::Char("i".into()),
            ],
        );
    }

    #[test]
    fn back_to_back_arrows() {
        let mut p = InputParser::new();
        let out = feed(&mut p, &[0x1B, b'[', b'A', 0x1B, b'[', b'B']);
        assert_eq!(
            out,
            vec![Owned::Ctrl(ControlKind::Up), Owned::Ctrl(ControlKind::Down)],
        );
    }

    #[test]
    fn ascii_letters() {
        let mut p = InputParser::new();
        let out = feed(&mut p, b"abc");
        assert_eq!(
            out,
            vec![
                Owned::Char("a".into()),
                Owned::Char("b".into()),
                Owned::Char("c".into()),
            ],
        );
    }

    #[test]
    fn ascii_space_and_punctuation() {
        let mut p = InputParser::new();
        let out = feed(&mut p, b" !,.");
        assert_eq!(
            out,
            vec![
                Owned::Char(" ".into()),
                Owned::Char("!".into()),
                Owned::Char(",".into()),
                Owned::Char(".".into()),
            ],
        );
    }

    #[test]
    fn multibyte_utf8_two_byte() {
        let mut p = InputParser::new();
        assert!(p.push_byte(0xC3).is_none());
        match p.push_byte(0xA9) {
            Some(InputType::Char(s)) => assert_eq!(s, "é"),
            other => panic!("expected Char(\"é\"), got {:?}", other),
        }
    }

    #[test]
    fn multibyte_utf8_three_byte() {
        let mut p = InputParser::new();
        assert!(p.push_byte(0xE6).is_none());
        assert!(p.push_byte(0x97).is_none());
        match p.push_byte(0xA5) {
            Some(InputType::Char(s)) => assert_eq!(s, "日"),
            other => panic!("expected Char(\"日\"), got {:?}", other),
        }
    }

    #[test]
    fn multibyte_utf8_four_byte() {
        let mut p = InputParser::new();
        assert!(p.push_byte(0xF0).is_none());
        assert!(p.push_byte(0x9F).is_none());
        assert!(p.push_byte(0xA6).is_none());
        match p.push_byte(0x80) {
            Some(InputType::Char(s)) => assert_eq!(s, "🦀"),
            other => panic!("expected Char(\"🦀\"), got {:?}", other),
        }
    }

    #[test]
    fn realistic_mixed_session() {
        let mut p = InputParser::new();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"hi");
        bytes.extend_from_slice(b"\r\n");
        bytes.extend_from_slice(&[0x1B, b'[', b'A']);
        bytes.push(0x7F);

        let out = feed(&mut p, &bytes);
        assert_eq!(
            out,
            vec![
                Owned::Char("h".into()),
                Owned::Char("i".into()),
                Owned::Ctrl(ControlKind::Enter),
                Owned::Ctrl(ControlKind::Up),
                Owned::Ctrl(ControlKind::Backspace),
            ],
        );
    }

    #[test]
    fn sub_space_control_bytes_other_than_known_are_ignored() {
        let mut p = InputParser::new();
        let out = feed(&mut p, &[0x01, 0x02, 0x05, 0x06]);
        assert_eq!(out, vec![]);
    }

    #[test]
    fn parser_state_recovers_after_lone_esc() {
        let mut p = InputParser::new();
        let out = feed(&mut p, &[0x1B, b'a']);
        assert_eq!(out, vec![Owned::Char("a".into())]);
    }
}
