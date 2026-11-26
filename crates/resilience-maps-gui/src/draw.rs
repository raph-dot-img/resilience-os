//! Drawing a [`Scene`] with iced's canvas (anti-aliased by tiny-skia), and
//! the mouse on the map: drag to move, wheel to zoom around the pointer.

use iced::mouse::{self, Cursor};
use iced::widget::canvas::{self, Fill, Frame, Geometry, LineDash, Path, Stroke, Text, fill};
use iced::{Color, Font, Pixels, Point, Rectangle, Renderer, Theme, alignment};
use resilience_core::maps::canvas::{Colour, LinePattern};
use resilience_maps::scene::{EDGE, Scene, Shape};

use crate::Message;

/// The map's colours: the eight terminal colours of the style table,
/// redrawn calmly for a screen (same meaning, ADR 0016 §3).
pub fn colour(c: Colour) -> Color {
    match c {
        // Land.
        Colour::Black => Color::from_rgb8(0x1f, 0x23, 0x2a),
        Colour::Red => Color::from_rgb8(0xe5, 0x6b, 0x6b),
        // Woods, parks.
        Colour::Green => Color::from_rgb8(0x3f, 0x8f, 0x5a),
        // Main roads.
        Colour::Yellow => Color::from_rgb8(0xf0, 0xc0, 0x5a),
        // Sea, lakes.
        Colour::Blue => Color::from_rgb8(0x1d, 0x4e, 0x89),
        Colour::Magenta => Color::from_rgb8(0xc0, 0x84, 0xfc),
        // Rivers, streams.
        Colour::Cyan => Color::from_rgb8(0x5c, 0xb8, 0xe6),
        // Small roads, names.
        Colour::White => Color::from_rgb8(0xdc, 0xe0, 0xe6),
    }
}

pub const LABEL_SIZE: f32 = 13.0;
/// The label grid's cell on screen (the scene places labels on it).
pub const CELL: (f32, f32) = (7.6, 17.0);

/// The canvas program: the scene, drawn `zoom` times larger than its own
/// pixels (the planner's views are at most 1023 pixels wide).
pub struct MapCanvas<'a> {
    pub scene: &'a Scene,
    pub zoom: f32,
    pub font: Font,
    /// The drawn map, kept until the scene changes (redrawing thousands of
    /// shapes at every tick would starve the input).
    pub cache: &'a canvas::Cache,
}

#[derive(Default)]
pub struct DragState {
    last: Option<Point>,
}

impl canvas::Program<Message> for MapCanvas<'_> {
    type State = DragState;

    fn update(
        &self,
        state: &mut DragState,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: Cursor,
    ) -> Option<canvas::Action<Message>> {
        let iced::Event::Mouse(ev) = event else {
            return None;
        };
        match ev {
            mouse::Event::ButtonPressed(mouse::Button::Left) => {
                state.last = cursor.position_in(bounds);
                state.last.map(|_| canvas::Action::capture())
            }
            mouse::Event::ButtonReleased(mouse::Button::Left) => {
                state.last = None;
                None
            }
            mouse::Event::CursorMoved { .. } => {
                let (Some(last), Some(now)) = (state.last, cursor.position_in(bounds)) else {
                    return None;
                };
                state.last = Some(now);
                let (dx, dy) = (now.x - last.x, now.y - last.y);
                Some(
                    canvas::Action::publish(Message::Drag(
                        f64::from(dx / self.zoom),
                        f64::from(dy / self.zoom),
                    ))
                    .and_capture(),
                )
            }
            mouse::Event::WheelScrolled { delta } => {
                let p = cursor.position_in(bounds)?;
                let lines = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => *y / 40.0,
                };
                if !lines.is_finite() || lines == 0.0 {
                    return None;
                }
                let (cx, cy) = self.centre(bounds);
                Some(
                    canvas::Action::publish(Message::Wheel(
                        f64::from(lines.clamp(-5.0, 5.0)),
                        f64::from((p.x - cx) / self.zoom),
                        f64::from((p.y - cy) / self.zoom),
                    ))
                    .and_capture(),
                )
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &DragState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: Cursor,
    ) -> Vec<Geometry> {
        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            self.paint(frame, bounds);
        });
        vec![geometry]
    }

    fn mouse_interaction(
        &self,
        state: &DragState,
        bounds: Rectangle,
        cursor: Cursor,
    ) -> mouse::Interaction {
        if state.last.is_some() {
            mouse::Interaction::Grabbing
        } else if cursor.is_over(bounds) {
            mouse::Interaction::Grab
        } else {
            mouse::Interaction::default()
        }
    }
}

impl MapCanvas<'_> {
    /// Top left of the scene, centred in `bounds`.
    fn origin(&self, bounds: Rectangle) -> (f32, f32) {
        let w = self.scene.width as f32 * self.zoom;
        let h = self.scene.height as f32 * self.zoom;
        ((bounds.width - w) / 2.0, (bounds.height - h) / 2.0)
    }

    fn centre(&self, bounds: Rectangle) -> (f32, f32) {
        let (ox, oy) = self.origin(bounds);
        (
            ox + self.scene.crosshair.0 as f32 * self.zoom,
            oy + self.scene.crosshair.1 as f32 * self.zoom,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn line(
    frame: &mut Frame,
    points: &[(f64, f64)],
    at: &impl Fn((f64, f64)) -> Point,
    c: Color,
    width: f32,
    pattern: LinePattern,
    phase: f64,
    k: f32,
) {
    if points.len() < 2 {
        return;
    }
    let path = Path::new(|b| {
        b.move_to(at(points[0]));
        for p in &points[1..] {
            b.line_to(at(*p));
        }
    });
    let (on, off) = pattern.period();
    let segs = [on as f32 * 2.0 * k, off as f32 * 2.0 * k];
    let mut stroke = Stroke::default()
        .with_color(c)
        .with_width(width)
        .with_line_cap(canvas::LineCap::Round)
        .with_line_join(canvas::LineJoin::Round);
    if pattern != LinePattern::Solid {
        stroke.line_dash = LineDash {
            segments: &segs,
            offset: (phase as f32 * 2.0 * k) as usize,
        };
    }
    frame.stroke(&path, stroke);
}

/// A label with a dark halo so it reads over any colour.
fn label(frame: &mut Frame, p: &Point, t: &str, c: Color, font: Font, marker: bool) {
    let size = if marker { LABEL_SIZE + 1.0 } else { LABEL_SIZE };
    let mut base = Text {
        content: t.to_string(),
        position: *p,
        color: Color::from_rgba8(0x11, 0x13, 0x17, 0.85),
        size: Pixels(size),
        font,
        align_x: alignment::Horizontal::Left.into(),
        align_y: alignment::Vertical::Top,
        ..Text::default()
    };
    for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
        base.position = Point::new(p.x + dx, p.y + dy);
        frame.fill_text(base.clone());
    }
    base.position = *p;
    base.color = c;
    frame.fill_text(base);
}

impl MapCanvas<'_> {
    fn paint(&self, frame: &mut Frame, bounds: Rectangle) {
        let s = self.scene;
        let k = self.zoom;
        let (ox, oy) = self.origin(bounds);
        let at = |p: (f64, f64)| Point::new(ox + p.0 as f32 * k, oy + p.1 as f32 * k);
        // The scene is centred and may be a pixel or two smaller than the
        // canvas (odd sizes): its background fills all of it.
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), colour(s.background));
        let ring_path = |rings: &[Vec<(f64, f64)>]| {
            Path::new(|b| {
                for r in rings {
                    let mut it = r.iter();
                    if let Some(first) = it.next() {
                        b.move_to(at(*first));
                        for p in it {
                            b.line_to(at(*p));
                        }
                        b.close();
                    }
                }
            })
        };
        for shape in &s.shapes {
            match shape {
                Shape::Fill { rings, colour: c } => frame.fill(
                    &ring_path(rings),
                    Fill {
                        style: canvas::Style::Solid(colour(*c)),
                        rule: fill::Rule::EvenOdd,
                    },
                ),
                Shape::Cover { rings } => frame.fill(
                    &ring_path(rings),
                    Fill {
                        style: canvas::Style::Solid(colour(Colour::Black)),
                        rule: fill::Rule::EvenOdd,
                    },
                ),
                Shape::Line {
                    points,
                    colour: c,
                    pattern,
                    phase,
                    road,
                } => {
                    let width = if *road { 1.8 } else { 1.0 } * k.max(1.0);
                    line(frame, points, &at, colour(*c), width, *pattern, *phase, k);
                }
                Shape::Edge { points } => {
                    line(frame, points, &at, colour(EDGE.0), 1.2, EDGE.1, 0.0, k);
                }
            }
        }
        if let Some(rings) = &s.veil {
            let full = vec![
                (0.0, 0.0),
                (f64::from(s.width), 0.0),
                (f64::from(s.width), f64::from(s.height)),
                (0.0, f64::from(s.height)),
            ];
            let mut all = vec![full];
            all.extend(rings.iter().cloned());
            let mut veil = colour(Colour::Black);
            veil.a = 0.72;
            frame.fill(
                &ring_path(&all),
                Fill {
                    style: canvas::Style::Solid(veil),
                    rule: fill::Rule::EvenOdd,
                },
            );
        }
        for l in &s.labels {
            let c = colour(l.colour);
            if let Some((p, ch)) = &l.marker {
                label(frame, &at(*p), &ch.to_string(), c, self.font, true);
            }
            if let Some((p, t)) = &l.text {
                label(frame, &at(*p), t, c, self.font, false);
            }
        }
        // The crosshair: the place the readout's coordinates name.
        let c = at(s.crosshair);
        let cross = Path::new(|b| {
            b.move_to(Point::new(c.x - 9.0, c.y));
            b.line_to(Point::new(c.x + 9.0, c.y));
            b.move_to(Point::new(c.x, c.y - 9.0));
            b.line_to(Point::new(c.x, c.y + 9.0));
        });
        frame.stroke(
            &cross,
            Stroke::default().with_color(Color::BLACK).with_width(4.0),
        );
        frame.stroke(
            &cross,
            Stroke::default().with_color(Color::WHITE).with_width(2.0),
        );
    }
}
