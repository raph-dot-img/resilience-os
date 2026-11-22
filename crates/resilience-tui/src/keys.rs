//! crossterm key events -> the shell's key events (ADR 0026).

use ratatui::crossterm::event as ct;
use resilience_shell::input::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

pub fn from_crossterm(k: ct::KeyEvent) -> KeyEvent {
    let code = match k.code {
        ct::KeyCode::Backspace => KeyCode::Backspace,
        ct::KeyCode::Enter => KeyCode::Enter,
        ct::KeyCode::Left => KeyCode::Left,
        ct::KeyCode::Right => KeyCode::Right,
        ct::KeyCode::Up => KeyCode::Up,
        ct::KeyCode::Down => KeyCode::Down,
        ct::KeyCode::Home => KeyCode::Home,
        ct::KeyCode::End => KeyCode::End,
        ct::KeyCode::PageUp => KeyCode::PageUp,
        ct::KeyCode::PageDown => KeyCode::PageDown,
        ct::KeyCode::Tab => KeyCode::Tab,
        ct::KeyCode::BackTab => KeyCode::BackTab,
        ct::KeyCode::Delete => KeyCode::Delete,
        ct::KeyCode::Insert => KeyCode::Insert,
        ct::KeyCode::F(n) => KeyCode::F(n),
        ct::KeyCode::Char(c) => KeyCode::Char(c),
        ct::KeyCode::Esc => KeyCode::Esc,
        _ => KeyCode::Other,
    };
    let mut modifiers = KeyModifiers::NONE;
    if k.modifiers.contains(ct::KeyModifiers::SHIFT) {
        modifiers |= KeyModifiers::SHIFT;
    }
    if k.modifiers.contains(ct::KeyModifiers::CONTROL) {
        modifiers |= KeyModifiers::CONTROL;
    }
    if k.modifiers.contains(ct::KeyModifiers::ALT) {
        modifiers |= KeyModifiers::ALT;
    }
    let kind = match k.kind {
        ct::KeyEventKind::Press => KeyEventKind::Press,
        ct::KeyEventKind::Repeat => KeyEventKind::Repeat,
        ct::KeyEventKind::Release => KeyEventKind::Release,
    };
    KeyEvent {
        code,
        modifiers,
        kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_c_and_release() {
        let mut k = ct::KeyEvent::new(ct::KeyCode::Char('c'), ct::KeyModifiers::CONTROL);
        let s = from_crossterm(k);
        assert_eq!(s.code, KeyCode::Char('c'));
        assert!(s.modifiers.contains(KeyModifiers::CONTROL));
        assert_eq!(s.kind, KeyEventKind::Press);
        k.kind = ct::KeyEventKind::Release;
        assert_eq!(from_crossterm(k).kind, KeyEventKind::Release);
        let f = ct::KeyEvent::new(ct::KeyCode::CapsLock, ct::KeyModifiers::NONE);
        assert_eq!(from_crossterm(f).code, KeyCode::Other);
    }
}
