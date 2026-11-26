//! The viewer's text screens (search, nearby, go-to, information, legend,
//! help, "no map"): the terminal viewer's renderer draws them into an
//! in-memory buffer, shown as styled monospace text. Keys work exactly as
//! in the terminal viewer.

use iced::widget::text::{LineHeight, Span};
use iced::widget::{column, rich_text};
use iced::{Color, Element, Font, Pixels};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::{Color as TColor, Modifier};
use resilience_maps::app::MapApp;
use resilience_maps::ui;

pub const FONT_SIZE: f32 = 15.0;
/// DejaVu Sans Mono's advance is 0.602 em.
const CELL_W: f32 = FONT_SIZE * 0.602;
const CELL_H: f32 = FONT_SIZE * 1.2;

#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub text: String,
    pub fg: TColor,
    pub bg: TColor,
}

/// The grid that fits `w` x `h` pixels (at least the viewer's minimum).
pub fn grid_for(w: f32, h: f32) -> (u16, u16) {
    let cols = (w / CELL_W).floor().max(80.0).clamp(80.0, 300.0) as u16;
    let rows = (h / CELL_H).floor().max(24.0).clamp(24.0, 120.0) as u16;
    (cols, rows)
}

pub fn render(app: &mut MapApp, cols: u16, rows: u16) -> Vec<Vec<Run>> {
    let Ok(mut term) = Terminal::new(TestBackend::new(cols, rows));
    if term.draw(|f| ui::draw(f, app)).is_err() {
        return Vec::new();
    }
    let buf = term.backend().buffer();
    let mut lines = Vec::with_capacity(rows as usize);
    for y in 0..rows {
        let mut runs: Vec<Run> = Vec::new();
        for x in 0..cols {
            let Some(cell) = buf.cell((x, y)) else {
                continue;
            };
            let (mut fg, mut bg) = (cell.fg, cell.bg);
            if cell.modifier.contains(Modifier::REVERSED) {
                std::mem::swap(&mut fg, &mut bg);
                if fg == TColor::Reset {
                    fg = TColor::Black;
                }
                if bg == TColor::Reset {
                    bg = TColor::Gray;
                }
            }
            match runs.last_mut() {
                Some(r) if r.fg == fg && r.bg == bg => r.text.push_str(cell.symbol()),
                _ => runs.push(Run {
                    text: cell.symbol().to_string(),
                    fg,
                    bg,
                }),
            }
        }
        lines.push(runs);
    }
    lines
}

fn color(c: TColor, accent: Color, fg: bool) -> Option<Color> {
    Some(match c {
        TColor::Reset => {
            if fg {
                Color::from_rgb8(0xf1, 0xf3, 0xf5)
            } else {
                return None;
            }
        }
        TColor::Black => Color::from_rgb8(0x11, 0x13, 0x17),
        TColor::Red | TColor::LightRed => Color::from_rgb8(0xf8, 0x71, 0x71),
        TColor::Green | TColor::LightGreen => accent,
        TColor::Yellow | TColor::LightYellow => Color::from_rgb8(0xfb, 0xbf, 0x24),
        TColor::Blue | TColor::LightBlue => Color::from_rgb8(0x60, 0xa5, 0xfa),
        TColor::Magenta | TColor::LightMagenta => Color::from_rgb8(0xc0, 0x84, 0xfc),
        TColor::Cyan | TColor::LightCyan => Color::from_rgb8(0x38, 0xbd, 0xf8),
        TColor::DarkGray => Color::from_rgb8(0x9a, 0xa1, 0xab),
        _ => Color::from_rgb8(0xf1, 0xf3, 0xf5),
    })
}

pub fn view<'a, M: 'a>(lines: &'a [Vec<Run>], accent: Color, font: Font) -> Element<'a, M> {
    let rows: Vec<Element<'a, M>> = lines
        .iter()
        .map(|runs| {
            let spans: Vec<Span<'a, (), Font>> = runs
                .iter()
                .map(|r| {
                    let mut s = iced::widget::span(r.text.as_str())
                        .font(font)
                        .size(Pixels(FONT_SIZE))
                        .line_height(LineHeight::Absolute(Pixels(CELL_H)));
                    if let Some(c) = color(r.fg, accent, true) {
                        s = s.color(c);
                    }
                    if let Some(c) = color(r.bg, accent, false) {
                        s = s.background(c);
                    }
                    s
                })
                .collect();
            rich_text(spans).into()
        })
        .collect();
    column(rows).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_has_a_minimum_and_a_maximum() {
        assert_eq!(grid_for(10.0, 10.0), (80, 24));
        assert_eq!(grid_for(f32::NAN, f32::NAN), (80, 24));
        let (c, r) = grid_for(100_000.0, 100_000.0);
        assert_eq!((c, r), (300, 120));
    }
}
