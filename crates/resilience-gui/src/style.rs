//! Look of the GUI: one accent colour per theme, a dark calm background,
//! large rounded surfaces, the bundled Inter font (ADR 0026 §4).

use std::fs;
use std::path::PathBuf;

use iced::widget::{button, container, text, text_input};
use iced::{Background, Border, Color, Font, Shadow, Theme, font, keyboard};
use resilience_core::config::ThemeName;
use resilience_core::network::LinkLevel;
use resilience_shell::app::App;
use resilience_shell::input::{KeyCode, KeyEvent, KeyModifiers};

pub const UI_FONT: Font = Font::with_name("Inter Variable");
pub const UI_FONT_BOLD: Font = Font {
    weight: font::Weight::Semibold,
    ..UI_FONT
};
pub const MONO_FONT: Font = Font::with_name("DejaVu Sans Mono");

/// Directory with the bundled fonts (set by the session; no system fonts
/// are needed).
pub const FONTS_VAR: &str = "RESILIENCE_GUI_FONTS";

pub fn load_fonts() -> Vec<Vec<u8>> {
    let Some(dir) = std::env::var_os(FONTS_VAR).map(PathBuf::from) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut stack = vec![dir];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "ttf" || x == "otf")
                && let Ok(b) = fs::read(&p)
            {
                out.push(b);
            }
        }
    }
    out
}

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub bg: Color,
    pub surface: Color,
    pub surface_hover: Color,
    pub text: Color,
    pub dim: Color,
    pub accent: Color,
    pub ok: Color,
    pub warn: Color,
    pub error: Color,
}

impl Palette {
    pub fn of(t: ThemeName) -> Palette {
        let accent = match t {
            ThemeName::Green => Color::from_rgb8(0x4a, 0xde, 0x80),
            ThemeName::Amber => Color::from_rgb8(0xfb, 0xbf, 0x24),
            ThemeName::Cyan => Color::from_rgb8(0x38, 0xbd, 0xf8),
            ThemeName::Mono => Color::from_rgb8(0xe5, 0xe5, 0xe5),
        };
        Palette {
            bg: Color::from_rgb8(0x11, 0x13, 0x17),
            surface: Color::from_rgb8(0x1b, 0x1e, 0x24),
            surface_hover: Color::from_rgb8(0x25, 0x29, 0x31),
            text: Color::from_rgb8(0xf1, 0xf3, 0xf5),
            dim: Color::from_rgb8(0x9a, 0xa1, 0xab),
            accent,
            ok: Color::from_rgb8(0x4a, 0xde, 0x80),
            warn: Color::from_rgb8(0xfb, 0xbf, 0x24),
            error: Color::from_rgb8(0xf8, 0x71, 0x71),
        }
    }
}

pub fn theme(t: ThemeName) -> Theme {
    let p = Palette::of(t);
    Theme::custom(
        "Resilience",
        iced::theme::Palette {
            background: p.bg,
            text: p.text,
            primary: p.accent,
            success: p.ok,
            warning: p.warn,
            danger: p.error,
        },
    )
}

fn pal(t: &Theme) -> Palette {
    // Our theme's primary is the accent; the rest is fixed.
    let mut p = Palette::of(ThemeName::Green);
    p.accent = t.palette().primary;
    p
}

pub fn page(p: &Palette) -> container::Style {
    container::Style {
        background: Some(Background::Color(p.bg)),
        text_color: Some(p.text),
        ..container::Style::default()
    }
}

pub fn status_bar(p: &Palette) -> container::Style {
    container::Style {
        background: Some(Background::Color(p.surface)),
        text_color: Some(p.text),
        ..container::Style::default()
    }
}

pub fn card(p: &Palette) -> container::Style {
    container::Style {
        background: Some(Background::Color(p.surface)),
        text_color: Some(p.text),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 16.0.into(),
        },
        ..container::Style::default()
    }
}

/// A read-only input field look (the shell holds the text).
pub fn field(p: &Palette) -> container::Style {
    container::Style {
        background: Some(Background::Color(p.surface)),
        text_color: Some(p.text),
        border: Border {
            color: p.accent,
            width: 1.5,
            radius: 12.0.into(),
        },
        ..container::Style::default()
    }
}

pub fn dim_text(t: &Theme) -> text::Style {
    text::Style {
        color: Some(pal(t).dim),
    }
}

/// A card; `selected` = the keyboard's current row (accent border).
pub fn tile(t: &Theme, s: button::Status, selected: bool) -> button::Style {
    let p = pal(t);
    let hover = matches!(s, button::Status::Hovered | button::Status::Pressed);
    let bg = if hover || selected {
        p.surface_hover
    } else {
        p.surface
    };
    let border = if selected {
        p.accent
    } else if hover {
        Color { a: 0.5, ..p.accent }
    } else {
        Color::TRANSPARENT
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: p.text,
        border: Border {
            color: border,
            width: 1.5,
            radius: 16.0.into(),
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn row_button(t: &Theme, s: button::Status, selected: bool) -> button::Style {
    let mut st = tile(t, s, selected);
    st.border.radius = 10.0.into();
    st
}

pub fn ghost_button(t: &Theme, s: button::Status) -> button::Style {
    let p = pal(t);
    let bg = match s {
        button::Status::Hovered | button::Status::Pressed => p.surface_hover,
        _ => p.surface,
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: p.text,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 999.0.into(),
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn search_box(t: &Theme, s: text_input::Status) -> text_input::Style {
    let p = pal(t);
    let border = match s {
        text_input::Status::Focused { .. } => p.accent,
        _ => p.surface_hover,
    };
    text_input::Style {
        background: Background::Color(p.surface),
        border: Border {
            color: border,
            width: 1.5,
            radius: 999.0.into(),
        },
        icon: p.dim,
        placeholder: p.dim,
        value: p.text,
        selection: Color {
            a: 0.35,
            ..p.accent
        },
    }
}

/// Right side of the status bar: network, battery, time.
pub fn status_line(app: &App) -> String {
    let net = if app.probe.running.is_some() {
        "Checking the Internet\u{2026}"
    } else {
        match app.net.level {
            LinkLevel::Routed => "Network connected",
            LinkLevel::LocalLink => "Local network only",
            LinkLevel::Offline => "Offline",
        }
    };
    let mut parts = vec![net.to_string()];
    if let Some(b) = app.snapshot.batteries.first() {
        let cap = b
            .capacity
            .map(|c| format!("{c}%"))
            .unwrap_or_else(|| "?".into());
        let charging = if b.status == "Charging" {
            " charging"
        } else {
            ""
        };
        parts.push(format!("Battery {cap}{charging}"));
    }
    parts.push(resilience_shell::fmt::date_time(
        std::time::SystemTime::now(),
    ));
    parts.join("   \u{00b7}   ")
}

/// iced key press -> the shell's key event (None for keys it ignores).
pub fn key_from_iced(ev: &keyboard::Event) -> Option<KeyEvent> {
    use keyboard::key::Named;
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
            Named::Insert => KeyCode::Insert,
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
            Named::F2 => KeyCode::F(2),
            Named::F3 => KeyCode::F(3),
            Named::F4 => KeyCode::F(4),
            Named::F5 => KeyCode::F(5),
            Named::F6 => KeyCode::F(6),
            Named::F7 => KeyCode::F(7),
            Named::F8 => KeyCode::F(8),
            Named::F9 => KeyCode::F(9),
            Named::F10 => KeyCode::F(10),
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

/// A dialog button: `primary` is filled with the accent (or the error
/// colour when `danger`); `focused` is the one Enter presses.
pub fn dialog_button(
    t: &Theme,
    s: button::Status,
    primary: bool,
    danger: bool,
    focused: bool,
) -> button::Style {
    let p = pal(t);
    let hover = matches!(s, button::Status::Hovered | button::Status::Pressed);
    let fill = if danger { p.error } else { p.accent };
    let (bg, fg) = if primary {
        (
            Color {
                a: if hover { 0.85 } else { 1.0 },
                ..fill
            },
            p.bg,
        )
    } else if hover {
        (p.surface_hover, p.text)
    } else {
        (p.surface, p.text)
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: fg,
        border: Border {
            color: if focused { p.text } else { Color::TRANSPARENT },
            width: if focused { 2.0 } else { 0.0 },
            radius: 10.0.into(),
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

/// The dimmed layer behind a dialog.
pub fn scrim(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color {
            a: 0.72,
            ..Color::BLACK
        })),
        ..container::Style::default()
    }
}

/// The dialog card.
pub fn dialog(p: &Palette, error: bool) -> container::Style {
    container::Style {
        background: Some(Background::Color(p.surface)),
        text_color: Some(p.text),
        border: Border {
            color: if error { p.error } else { p.surface_hover },
            width: 1.0,
            radius: 16.0.into(),
        },
        // No shadow: tiny-skia draws it over the card.
        shadow: Shadow::default(),
        snap: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::keyboard::{Key, Location, Modifiers, key};

    fn press(key: Key, modifiers: Modifiers, text: Option<&str>) -> keyboard::Event {
        keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: key::Physical::Unidentified(key::NativeCode::Unidentified),
            location: Location::Standard,
            modifiers,
            text: text.map(Into::into),
            repeat: false,
        }
    }

    #[test]
    fn keys_map() {
        let e = key_from_iced(&press(
            Key::Named(key::Named::Escape),
            Modifiers::empty(),
            None,
        ));
        assert_eq!(e.map(|k| k.code), Some(KeyCode::Esc));
        let c = key_from_iced(&press(
            Key::Character("c".into()),
            Modifiers::CTRL,
            Some("\u{3}"),
        ))
        .expect("ctrl-c");
        assert_eq!(c.code, KeyCode::Char('c'));
        assert!(c.modifiers.contains(KeyModifiers::CONTROL));
        let q = key_from_iced(&press(
            Key::Character("a".into()),
            Modifiers::SHIFT,
            Some("A"),
        ));
        assert_eq!(q.map(|k| k.code), Some(KeyCode::Char('A')));
        let bt = key_from_iced(&press(Key::Named(key::Named::Tab), Modifiers::SHIFT, None));
        assert_eq!(bt.map(|k| k.code), Some(KeyCode::BackTab));
    }
}
