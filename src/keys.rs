//! Rune key events → the bytes a shell expects (xterm conventions).

use alacritty_terminal::term::TermMode;
use rune::{Key, KeyEvent};

/// Bytes for `ev`, or None when the terminal has nothing to send for it.
pub fn bytes(ev: &KeyEvent, mode: TermMode) -> Option<Vec<u8>> {
    let m = ev.mods;
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    // xterm modifier parameter: 1 + shift + 2·alt + 4·ctrl.
    let modp = 1 + m.shift as u8 + 2 * m.alt as u8 + 4 * m.ctrl as u8;

    let csi_letter = |c: char| -> Vec<u8> {
        if modp > 1 { format!("\x1b[1;{modp}{c}").into_bytes() }
        else if app_cursor { format!("\x1bO{c}").into_bytes() }
        else { format!("\x1b[{c}").into_bytes() }
    };
    let csi_tilde = |n: u8| -> Vec<u8> {
        if modp > 1 { format!("\x1b[{n};{modp}~").into_bytes() } else { format!("\x1b[{n}~").into_bytes() }
    };
    let alt = |mut b: Vec<u8>| -> Vec<u8> { if m.alt { b.insert(0, 0x1b); } b };

    Some(match &ev.key {
        Key::Text(s) => alt(s.as_bytes().to_vec()),
        // Ctrl (Cmd on macOS) + a character. Off macOS that's always Ctrl.
        Key::Command(c) => {
            if !m.ctrl { return None; } // ⌘-chords on macOS are app shortcuts, not shell input
            alt(vec![control_byte(*c)?])
        }
        Key::Enter => alt(b"\r".to_vec()),
        Key::Backspace => alt(if m.ctrl { vec![0x08] } else { vec![0x7f] }),
        Key::Tab => if m.shift { b"\x1b[Z".to_vec() } else { b"\t".to_vec() },
        Key::Escape => b"\x1b".to_vec(),
        Key::Up => csi_letter('A'),
        Key::Down => csi_letter('B'),
        Key::Right => csi_letter('C'),
        Key::Left => csi_letter('D'),
        Key::Home => csi_letter('H'),
        Key::End => csi_letter('F'),
        Key::Insert => csi_tilde(2),
        Key::Delete => csi_tilde(3),
        Key::PageUp => csi_tilde(5),
        Key::PageDown => csi_tilde(6),
        Key::Function(n) => match n {
            1..=4 => {
                let c = [b'P', b'Q', b'R', b'S'][(*n - 1) as usize] as char;
                if modp > 1 { format!("\x1b[1;{modp}{c}").into_bytes() } else { format!("\x1bO{c}").into_bytes() }
            }
            5 => csi_tilde(15),
            6..=10 => csi_tilde(11 + n),
            11 | 12 => csi_tilde(12 + n),
            _ => return None,
        },
        _ => return None,
    })
}

/// Ctrl+`c` as a C0 control byte: Ctrl+A is 0x01, Ctrl+[ is ESC, Ctrl+Space is NUL.
fn control_byte(c: char) -> Option<u8> {
    let c = c.to_ascii_lowercase();
    Some(match c {
        'a'..='z' => c as u8 - b'a' + 1,
        ' ' | '@' | '2' => 0,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '-' | '7' => 0x1f,
        '/' => 0x1f,
        '8' | '?' => 0x7f,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::Mods;

    fn ev(key: Key, mods: Mods) -> KeyEvent { KeyEvent { key, mods } }

    #[test]
    fn ctrl_c_is_etx() {
        let m = Mods { ctrl: true, cmd: true, ..Default::default() };
        assert_eq!(bytes(&ev(Key::Command('c'), m), TermMode::empty()), Some(vec![3]));
    }

    #[test]
    fn arrows_follow_app_cursor_mode() {
        let none = Mods::default();
        assert_eq!(bytes(&ev(Key::Up, none), TermMode::empty()).unwrap(), b"\x1b[A");
        assert_eq!(bytes(&ev(Key::Up, none), TermMode::APP_CURSOR).unwrap(), b"\x1bOA");
        let ctrl = Mods { ctrl: true, ..Default::default() };
        assert_eq!(bytes(&ev(Key::Right, ctrl), TermMode::empty()).unwrap(), b"\x1b[1;5C");
    }

    #[test]
    fn alt_prefixes_escape() {
        let alt = Mods { alt: true, ..Default::default() };
        assert_eq!(bytes(&ev(Key::Text("b".into()), alt), TermMode::empty()).unwrap(), b"\x1bb");
    }
}
