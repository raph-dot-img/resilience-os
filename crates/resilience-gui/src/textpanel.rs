//! Screens without a graphical view yet: the terminal renderer draws them
//! into an in-memory buffer, shown here as styled monospace text. Keys
//! work exactly as in the TUI.

use iced::widget::text::Span;
use iced::widget::{column, rich_text};
use iced::{Color, Element, Font, Pixels, font};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::{Color as TColor, Modifier};
use resilience_shell::app::App;

use crate::style::{MONO_FONT, Palette};

pub const FONT_SIZE: f32 = 15.0;
/// DejaVu Sans Mono's advance is 0.602 em.
const CELL_W: f32 = FONT_SIZE * 0.602;
const CELL_H: f32 = FONT_SIZE * 1.2;

#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub text: String,
    pub fg: TColor,
    pub bg: TColor,
    pub bold: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Screen {
    pub lines: Vec<Vec<Run>>,
}

/// Grid that fits `w` x `h` pixels (at least 80x24, the TUI's minimum;
/// `max` first so a NaN size gives the minimum).
pub fn grid_for(w: f32, h: f32) -> (u16, u16) {
    let cols = (w / CELL_W).floor().max(80.0).clamp(80.0, 400.0) as u16;
    let rows = (h / CELL_H).floor().max(24.0).clamp(24.0, 200.0) as u16;
    (cols, rows)
}

pub fn render(app: &mut App, cols: u16, rows: u16) -> Screen {
    let Ok(mut term) = Terminal::new(TestBackend::new(cols, rows));
    if term
        .draw(|f| resilience_tui::ui::draw_panel(f, app))
        .is_err()
    {
        return Screen::default();
    }
    let buf = term.backend().buffer();
    let mut lines = Vec::with_capacity(rows as usize);
    for y in 0..rows {
        let mut runs: Vec<Run> = Vec::new();
        for x in 0..cols {
            let Some(cell) = buf.cell((x, y)) else {
                continue;
            };
            let mut fg = cell.fg;
            let mut bg = cell.bg;
            if cell.modifier.contains(Modifier::REVERSED) {
                std::mem::swap(&mut fg, &mut bg);
                if fg == TColor::Reset {
                    fg = TColor::Black;
                }
                if bg == TColor::Reset {
                    bg = TColor::Gray;
                }
            }
            let bold = cell.modifier.contains(Modifier::BOLD);
            match runs.last_mut() {
                Some(r) if r.fg == fg && r.bg == bg && r.bold == bold => {
                    r.text.push_str(cell.symbol())
                }
                _ => runs.push(Run {
                    text: cell.symbol().to_string(),
                    fg,
                    bg,
                    bold,
                }),
            }
        }
        lines.push(runs);
    }
    Screen { lines }
}

fn color(c: TColor, p: &Palette, fg: bool) -> Option<Color> {
    Some(match c {
        TColor::Reset => {
            if fg {
                p.text
            } else {
                return None;
            }
        }
        TColor::Black => p.bg,
        TColor::Red | TColor::LightRed => p.error,
        TColor::Green | TColor::LightGreen => p.ok,
        TColor::Yellow | TColor::LightYellow => p.warn,
        TColor::Blue | TColor::LightBlue => Color::from_rgb8(0x60, 0xa5, 0xfa),
        TColor::Magenta | TColor::LightMagenta => Color::from_rgb8(0xc0, 0x84, 0xfc),
        TColor::Cyan | TColor::LightCyan => p.accent,
        TColor::Gray | TColor::White => p.text,
        TColor::DarkGray => p.dim,
        _ => p.text,
    })
}

pub fn view<'a, M: 'a>(screen: &'a Screen, p: &Palette) -> Element<'a, M> {
    let lines: Vec<Element<'a, M>> = screen
        .lines
        .iter()
        .map(|runs| {
            let spans: Vec<Span<'a, (), Font>> = runs
                .iter()
                .map(|r| {
                    let mut s = iced::widget::span(r.text.as_str())
                        .font(if r.bold {
                            Font {
                                weight: font::Weight::Bold,
                                ..MONO_FONT
                            }
                        } else {
                            MONO_FONT
                        })
                        .size(Pixels(FONT_SIZE))
                        .line_height(iced::widget::text::LineHeight::Absolute(Pixels(CELL_H)));
                    if let Some(c) = color(r.fg, p, true) {
                        s = s.color(c);
                    }
                    if let Some(c) = color(r.bg, p, false) {
                        s = s.background(c);
                    }
                    s
                })
                .collect();
            rich_text(spans).into()
        })
        .collect();
    column(lines).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_has_a_minimum() {
        assert_eq!(grid_for(10.0, 10.0), (80, 24));
        let (c, r) = grid_for(1366.0, 700.0);
        assert!(c > 140 && r > 30, "{c}x{r}");
    }
}
