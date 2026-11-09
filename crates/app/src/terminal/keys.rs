//! Keystrokes to the bytes an xterm sends.

use gpui_kit::Keystroke;

use super::ghostty::Mode;

/// None for keys a terminal has no bytes for (Cmd chords, lone modifiers).
pub fn bytes(k: &Keystroke, mode: Mode) -> Option<Vec<u8>> {
    let m = &k.modifiers;
    if m.platform {
        return None;
    }
    // xterm's modifier parameter: 1 + Shift 1, Alt 2, Ctrl 4.
    let p = 1 + m.shift as u8 + 2 * m.alt as u8 + 4 * m.control as u8;
    let app = mode.app_cursor;
    let csi = |s: &str| format!("\x1b[{s}").into_bytes();
    let tilde = |n: u8| match p {
        1 => csi(&format!("{n}~")),
        _ => csi(&format!("{n};{p}~")),
    };
    // Arrows, Home, End and F1-F4: SS3 in application mode, CSI 1;p with modifiers.
    let letter = |c: char, ss3: bool| match p {
        1 if ss3 => format!("\x1bO{c}").into_bytes(),
        1 => csi(&c.to_string()),
        _ => csi(&format!("1;{p}{c}")),
    };
    let alt = |b: &[u8]| [&b"\x1b"[..m.alt as usize], b].concat();
    Some(match k.key.as_str() {
        "enter" => alt(b"\r"),
        "tab" if m.shift => csi("Z"),
        "tab" => b"\t".to_vec(),
        "escape" => b"\x1b".to_vec(),
        "backspace" if m.control => alt(b"\x08"),
        "backspace" => alt(b"\x7f"),
        "space" if m.control => b"\0".to_vec(),
        "up" => letter('A', app),
        "down" => letter('B', app),
        "right" => letter('C', app),
        "left" => letter('D', app),
        "home" => letter('H', app),
        "end" => letter('F', app),
        "f1" => letter('P', true),
        "f2" => letter('Q', true),
        "f3" => letter('R', true),
        "f4" => letter('S', true),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        f if f.len() > 1 && f.starts_with('f') => {
            let n: u8 = f[1..].parse().ok()?;
            const CODES: [u8; 8] = [15, 17, 18, 19, 20, 21, 23, 24];
            tilde(*CODES.get(usize::from(n.checked_sub(5)?))?)
        }
        key if m.control => {
            let c = key.chars().next().filter(|_| key.chars().count() == 1)?;
            let b = match c.to_ascii_lowercase() {
                c @ 'a'..='z' => c as u8 & 0x1f,
                '@' | '2' => 0,
                '[' | '3' => 0x1b,
                '\\' | '4' => 0x1c,
                ']' | '5' => 0x1d,
                '^' | '6' => 0x1e,
                '_' | '-' | '/' | '7' => 0x1f,
                '8' => 0x7f,
                _ => return None,
            };
            alt(&[b])
        }
        // Option composes characters on macOS, as in Terminal.app; Alt is Meta elsewhere.
        _ if m.alt && !cfg!(target_os = "macos") => alt(k.key.as_bytes()),
        _ => k.key_char.as_ref()?.as_bytes().to_vec(),
    })
}

/// What a paste writes: bracketed when the app asked for it, newlines as Enter.
pub fn paste(text: &str, mode: Mode) -> Vec<u8> {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    if mode.bracketed_paste {
        // A paste must not end the bracket early.
        let text = text.replace("\x1b[201~", "");
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &'static str) -> Vec<u8> {
        key_in(s, Mode::default())
    }

    fn key_in(s: &'static str, mode: Mode) -> Vec<u8> {
        let mut k = Keystroke::parse(s).unwrap();
        if k.key_char.is_none() && k.key.chars().count() == 1 && !k.modifiers.control {
            k.key_char = Some(if k.modifiers.shift {
                k.key.to_uppercase()
            } else {
                k.key.clone()
            });
        }
        bytes(&k, mode).unwrap_or_default()
    }

    #[test]
    fn plain_keys_send_their_characters() {
        assert_eq!(key("a"), b"a");
        assert_eq!(key("shift-a"), b"A");
        assert_eq!(key("enter"), b"\r");
        assert_eq!(key("backspace"), b"\x7f");
        assert_eq!(key("tab"), b"\t");
        assert_eq!(key("shift-tab"), b"\x1b[Z");
        assert_eq!(key("escape"), b"\x1b");
    }

    #[test]
    fn ctrl_letters_are_control_codes() {
        assert_eq!(key("ctrl-c"), [3]);
        assert_eq!(key("ctrl-d"), [4]);
        assert_eq!(key("ctrl-["), [0x1b]);
        assert_eq!(key("ctrl-space"), [0]);
        assert_eq!(key("ctrl-alt-c"), b"\x1b\x03");
    }

    #[test]
    fn arrows_follow_the_cursor_mode_and_modifiers() {
        assert_eq!(key("up"), b"\x1b[A");
        let app = Mode {
            app_cursor: true,
            ..Mode::default()
        };
        assert_eq!(key_in("up", app), b"\x1bOA");
        assert_eq!(key("ctrl-left"), b"\x1b[1;5D");
        assert_eq!(key("shift-end"), b"\x1b[1;2F");
    }

    #[test]
    fn function_and_editing_keys() {
        assert_eq!(key("f1"), b"\x1bOP");
        assert_eq!(key("f5"), b"\x1b[15~");
        assert_eq!(key("f12"), b"\x1b[24~");
        assert_eq!(key("shift-f5"), b"\x1b[15;2~");
        assert_eq!(key("delete"), b"\x1b[3~");
        assert_eq!(key("pageup"), b"\x1b[5~");
    }

    #[test]
    fn cmd_chords_have_no_bytes() {
        assert_eq!(key("cmd-c"), b"");
    }

    #[test]
    fn paste_brackets_when_asked_and_sends_newlines_as_enter() {
        assert_eq!(paste("a\nb", Mode::default()), b"a\rb");
        let bracketed = Mode {
            bracketed_paste: true,
            ..Mode::default()
        };
        assert_eq!(paste("a\x1b[201~b", bracketed), b"\x1b[200~ab\x1b[201~");
    }
}
