//! The Resilience pulse icon (assets/logo/resilience-icon.svg, by the
//! owner), drawn as vectors from the SVG's own coordinates.

use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke, path};
use iced::{Color, Point, Rectangle, Renderer, Theme, mouse};

/// The icon's drawing box in SVG units (x 0..310, y 10..150).
const W: f32 = 310.0;
const H: f32 = 140.0;
const Y0: f32 = 10.0;

pub struct Pulse {
    pub color: Color,
}

impl<M> canvas::Program<M> for Pulse {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let k = (bounds.width / W).min(bounds.height / H);
        let p = |x: f32, y: f32| Point::new(x * k, (y - Y0) * k);
        let base = Path::line(p(0.0, 80.0), p(300.0, 80.0));
        frame.stroke(
            &base,
            Stroke::default()
                .with_color(Color {
                    a: 0.25,
                    ..self.color
                })
                .with_width((2.0 * k).max(1.0)),
        );
        let line = Path::new(|b: &mut path::Builder| {
            b.move_to(p(10.0, 60.0));
            for (x, y) in [
                (70.0, 60.0),
                (90.0, 140.0),
                (110.0, 95.0),
                (130.0, 95.0),
                (150.0, 20.0),
                (170.0, 110.0),
                (190.0, 70.0),
                (230.0, 70.0),
            ] {
                b.line_to(p(x, y));
            }
            b.bezier_curve_to(p(260.0, 70.0), p(280.0, 40.0), p(300.0, 30.0));
        });
        frame.stroke(
            &line,
            Stroke::default()
                .with_color(self.color)
                .with_width((7.0 * k).max(1.5))
                .with_line_cap(canvas::LineCap::Round)
                .with_line_join(canvas::LineJoin::Round),
        );
        frame.fill(
            &Path::circle(p(300.0, 30.0), (8.0 * k).max(2.0)),
            self.color,
        );
        vec![frame.into_geometry()]
    }
}

/// The icon at `width` pixels wide.
pub fn icon<'a, M: 'a>(color: Color, width: f32) -> iced::Element<'a, M> {
    canvas::Canvas::new(Pulse { color })
        .width(width)
        .height(width * H / W)
        .into()
}
