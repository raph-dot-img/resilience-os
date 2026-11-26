//! iced keys to the viewer's keys (crossterm's, as `MapApp` takes them).

use iced::keyboard::{self, key::Named};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn from_iced(ev: &keyboard::Event) -> Option<KeyEvent> {
    let keyboard::Event::KeyPressed {
        key,
        modifiers,
        text,
        ..
    } = ev
    else {
        return None;
    };
    let mut m = KeyModifiers::NONE;
    if modifiers.shift() {
        m |= KeyModifiers::SHIFT;
    }
    if modifiers.control() {
        m |= KeyModifiers::CONTROL;
    }
    if modifiers.alt() {
        m |= KeyModifiers::ALT;
    }
    let code = match key.as_ref() {
        keyboard::Key::Named(n) => match n {
            Named::Enter => KeyCode::Enter,
            Named::Escape => KeyCode::Esc,
            Named::Backspace => KeyCode::Backspace,
            Named::Delete => KeyCode::Delete,
            Named::Tab if modifiers.shift() => KeyCode::BackTab,
            Named::Tab => KeyCode::Tab,
            Named::ArrowUp => KeyCode::Up,
            Named::ArrowDown => KeyCode::Down,
            Named::ArrowLeft => KeyCode::Left,
            Named::ArrowRight => KeyCode::Right,
            Named::Home => KeyCode::Home,
            Named::End => KeyCode::End,
            Named::PageUp => KeyCode::PageUp,
            Named::PageDown => KeyCode::PageDown,
            Named::Space => KeyCode::Char(' '),
            Named::F1 => KeyCode::F(1),
            _ => return None,
        },
        keyboard::Key::Character(c) => {
            // The text honours the layout and Shift; with Ctrl it is a
            // control character, so use the key itself then.
            let from_text = text
                .as_ref()
                .and_then(|t| t.chars().next())
                .filter(|ch| !ch.is_control());
            let ch = if modifiers.control() {
                c.chars().next()
            } else {
                from_text.or_else(|| c.chars().next())
            }?;
            KeyCode::Char(ch)
        }
        keyboard::Key::Unidentified => return None,
    };
    Some(KeyEvent::new(code, m))
}
