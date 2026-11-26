//! Packs and a view to vector shapes in pixels, for the graphical viewer
//! (ADR 0026 amendment 1). Pure, like [`render`](crate::render): the same
//! features, style rules, drawing order and coverage rules, but kept as
//! polygons, polylines and labels in view pixels instead of being
//! rasterised into block characters. The front end draws them.
//!
//! Differences from the block-character map, by design:
//!
//! - A detailed pack's area is not used as a pixel mask. At fine scales its
//!   coverage is first filled with the land colour ([`Shape::Cover`]), which
//!   hides the overview's coarse coastline there; the pack's own features
//!   (cut to its area by the builder) are drawn on top.
//! - Outside every detailed area at fine scales a veil is drawn
//!   ([`Scene::veil`]: the view with the areas as holes, even-odd).
//! - Labels are placed with the same greedy rules ([`label::place`]) on a
//!   grid of [`LABEL_CELL`] pixels, so the priorities and the crosshair's
//!   reserved cell behave as in the terminal.
//!
//! Every text comes from untrusted files and is already folded by
//! [`label::fold`] (the console repertoire, bounded length).

use std::collections::HashSet;

use resilience_core::maps::canvas::{Colour, LinePattern, Point};
use resilience_core::maps::geo::{View, Viewport};
use resilience_core::maps::label::{self, Candidate, CellRect};
use resilience_core::maps::mvt::{GeomType, PartRole};
use resilience_core::maps::style::{COVERAGE_EDGE, Draw, SEA, STREET_NAME_COLOUR, STREET_NAMES};

use crate::render::{
    self, Role, Source, area_pixels, collect, dash_phase, fine, label_text, pattern_origin,
};

/// Width and height in view pixels of one cell of the label grid when the
/// view is drawn at its own size (the front end passes its own cell size
/// when it enlarges the view).
pub const LABEL_CELL: (f64, f64) = (8.0, 16.0);

/// At most this many shapes in one scene (a hostile pack cannot make the
/// front end draw without bound; the terminal canvas has its own limits).
pub const MAX_SHAPES: usize = 200_000;

/// One thing to draw, in drawing order.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// A filled area (even-odd over its rings).
    Fill {
        rings: Vec<Vec<Point>>,
        colour: Colour,
    },
    /// A detailed pack's coverage, filled with the land colour before its
    /// features (fine scales only).
    Cover { rings: Vec<Vec<Point>> },
    /// A line; `road` lines are drawn wider.
    Line {
        points: Vec<Point>,
        colour: Colour,
        pattern: LinePattern,
        /// Phase of the dash pattern in view pixels (0 for solid lines).
        phase: f64,
        road: bool,
    },
    /// The edge of a detailed pack's coverage (coarse scales).
    Edge { points: Vec<Point> },
}

/// A placed label, in view pixels (top left of the text's first cell).
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub marker: Option<(Point, char)>,
    pub text: Option<(Point, String)>,
    pub colour: Colour,
}

/// What the front end draws for one view.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub width: u32,
    pub height: u32,
    pub background: Colour,
    pub shapes: Vec<Shape>,
    /// Rings of the detailed areas: outside them a veil is drawn (fine
    /// scales only; `None`: no veil).
    pub veil: Option<Vec<Vec<Point>>>,
    pub labels: Vec<Label>,
    /// The view's centre (the coordinates in the readout).
    pub crosshair: Point,
    /// The shape bound was reached: some features were left out.
    pub truncated: bool,
}

/// The pixel size of the viewport for a map of `width` x `height` pixels,
/// as the planner sees it: `(cols, rows)` of the terminal geometry (one
/// column per pixel, two pixels per row), so the same tiles are planned.
pub fn plan_size(width: u32, height: u32) -> (u16, u16) {
    // The viewport is (2 * (cols / 2) + 1) x 2 * (2 * (rows / 2) + 1)
    // pixels, at most MAX_VIEW_PIXELS each way.
    let max = resilience_core::maps::geo::MAX_VIEW_PIXELS;
    let w = width.clamp(1, max - 1) as u16;
    let h = (height / 2).clamp(1, max / 2 - 1) as u16;
    (w, h)
}

/// The scene of a map of `cols` x `rows` planner cells (see [`plan_size`]):
/// `cols` x `2 * rows` pixels (odd dimensions around the crosshair, as in
/// [`render::viewport`]).
pub fn scene(view: &View, cols: u16, rows: u16, sources: &[Source<'_>], cell: (f64, f64)) -> Scene {
    let cell = if cell.0.is_finite() && cell.1.is_finite() && cell.0 >= 1.0 && cell.1 >= 1.0 {
        cell
    } else {
        LABEL_CELL
    };
    let vp = render::viewport(view, cols, rows);
    let scale = view.scale();
    let fine = fine(scale);
    let (w, h) = (vp.width(), vp.height());
    let has_overview = sources.iter().any(|s| s.role == Role::Overview);
    let mut out = Scene {
        width: w,
        height: h,
        background: if has_overview { SEA } else { Colour::Black },
        shapes: Vec::new(),
        veil: None,
        labels: Vec::new(),
        crosshair: (f64::from(w) / 2.0, f64::from(h) / 2.0),
        truncated: false,
    };
    let origin = pattern_origin(&vp);
    let mut veil = Vec::new();
    let mut labels: Vec<Vec<(Candidate, Point)>> = Vec::with_capacity(sources.len());
    let mut areas: Vec<Option<Vec<Vec<Point>>>> = Vec::with_capacity(sources.len());
    let mut streets = HashSet::new();
    let mut edges = Vec::new();

    for s in sources {
        let mut own = Vec::new();
        let rings = match (s.role, s.area) {
            (Role::Overview, _) | (_, None) => None,
            (role, Some(area)) => {
                let rings = area_pixels(area, &vp);
                veil.extend(rings.iter().cloned());
                if !fine || role == Role::Approximate {
                    for r in &rings {
                        let mut closed = r.clone();
                        if let Some(first) = r.first() {
                            closed.push(*first);
                        }
                        edges.push(closed);
                    }
                }
                if fine {
                    push(
                        &mut out,
                        Shape::Cover {
                            rings: rings.clone(),
                        },
                    );
                }
                Some(rings)
            }
        };
        let items = collect(&s.tiles, &vp, scale);
        draw_items(
            &mut out,
            &vp,
            origin,
            scale,
            cell,
            &items,
            &mut own,
            &mut streets,
        );
        if let Some(r) = &rings {
            own.retain(|(_, p)| inside(r, *p));
        }
        labels.push(own);
        areas.push(rings);
    }
    for points in edges {
        push(&mut out, Shape::Edge { points });
    }
    if fine {
        out.veil = Some(veil);
    }

    // Labels inside the area of a source drawn later are left out.
    let mut all = Vec::new();
    for (i, own) in labels.into_iter().enumerate() {
        for (c, p) in own {
            let covered = areas[i + 1..]
                .iter()
                .zip(&sources[i + 1..])
                .any(|(a, s)| s.role != Role::Overview && a.as_ref().is_some_and(|a| inside(a, p)));
            if !covered {
                all.push(c);
            }
        }
    }
    let (lc, lr) = label_grid(w, h, cell);
    let (cc, cr) = (
        (out.crosshair.0 / cell.0) as u16,
        (out.crosshair.1 / cell.1) as u16,
    );
    let reserved = [CellRect {
        col: cc,
        row: cr,
        width: 1,
        height: 1,
    }];
    for p in label::place(&all, lc, lr, &reserved) {
        let at = |col: u16, row: u16| (f64::from(col) * cell.0, f64::from(row) * cell.1);
        out.labels.push(Label {
            marker: p.marker.map(|(c, r, ch)| (at(c, r), ch)),
            text: p.text.map(|(c, r, t)| (at(c, r), t)),
            colour: p.colour,
        });
    }
    out
}

fn label_grid(w: u32, h: u32, cell: (f64, f64)) -> (u16, u16) {
    (
        (f64::from(w) / cell.0).floor().clamp(1.0, 1024.0) as u16,
        (f64::from(h) / cell.1).floor().clamp(1.0, 1024.0) as u16,
    )
}

fn push(out: &mut Scene, s: Shape) {
    if out.shapes.len() < MAX_SHAPES {
        out.shapes.push(s);
    } else {
        out.truncated = true;
    }
}

/// Whether the bounding box of `points` meets the view (one pixel of
/// margin).
fn meets(w: u32, h: u32, points: &[Point]) -> bool {
    let (mut x0, mut y0, mut x1, mut y1) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for &(x, y) in points {
        if !(x.is_finite() && y.is_finite()) {
            return false;
        }
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    x1 >= -1.0 && y1 >= -1.0 && x0 <= f64::from(w) + 1.0 && y0 <= f64::from(h) + 1.0
}

#[allow(clippy::too_many_arguments)]
fn draw_items(
    out: &mut Scene,
    vp: &Viewport,
    origin: (f64, f64),
    scale: f64,
    cell: (f64, f64),
    items: &[render::Item<'_>],
    labels: &mut Vec<(Candidate, Point)>,
    streets: &mut HashSet<String>,
) {
    let (w, h) = (vp.width(), vp.height());
    let (lc, lr) = label_grid(w, h, cell);
    let street_names = STREET_NAMES.name_visible(scale);
    for it in items {
        let f = &it.feature;
        let t = it.transform;
        match it.rule.draw {
            Draw::Fill => {
                if f.geom_type() != GeomType::Polygon {
                    continue;
                }
                let rings: Vec<Vec<Point>> = f
                    .parts()
                    .filter(|p| matches!(p.role, PartRole::Exterior | PartRole::Interior))
                    .map(|p| p.points.iter().map(|q| t.apply(q.x, q.y)).collect())
                    .collect();
                if rings.iter().any(|r| meets(w, h, r)) {
                    push(
                        out,
                        Shape::Fill {
                            rings,
                            colour: it.rule.colour,
                        },
                    );
                }
            }
            Draw::Line(pattern) => {
                if f.geom_type() != GeomType::LineString {
                    continue;
                }
                for part in f.parts() {
                    let points: Vec<Point> =
                        part.points.iter().map(|q| t.apply(q.x, q.y)).collect();
                    if !meets(w, h, &points) {
                        continue;
                    }
                    let phase = if pattern == LinePattern::Solid {
                        0.0
                    } else {
                        dash_phase(&points, origin)
                    };
                    if it.road && street_names {
                        street_label(f, &points, cell, lc, lr, labels, streets);
                    }
                    push(
                        out,
                        Shape::Line {
                            points,
                            colour: it.rule.colour,
                            pattern,
                            phase,
                            road: it.road,
                        },
                    );
                }
            }
            Draw::Label(l) => {
                if f.geom_type() != GeomType::Point {
                    continue;
                }
                let mut text: Option<String> = None;
                for part in f.parts() {
                    for q in part.points {
                        let (x, y) = t.apply(q.x, q.y);
                        if !(x >= 0.0 && y >= 0.0 && x < f64::from(w) && y < f64::from(h)) {
                            continue;
                        }
                        let text = text.get_or_insert_with(|| label_text(&l, f, scale));
                        labels.push((
                            Candidate {
                                priority: l.priority,
                                col: (x / cell.0).floor() as i32,
                                row: (y / cell.1).floor() as i32,
                                marker: l.marker,
                                text: text.clone(),
                                colour: it.rule.colour,
                                keep_marker: l.keep_marker,
                            },
                            (x, y),
                        ));
                    }
                }
            }
        }
    }
}

fn street_label(
    f: &resilience_core::maps::mvt::Feature<'_>,
    points: &[Point],
    cell: (f64, f64),
    cols: u16,
    rows: u16,
    labels: &mut Vec<(Candidate, Point)>,
    streets: &mut HashSet<String>,
) {
    let Some(name) = f
        .attr("name")
        .and_then(|v| v.as_str())
        .and_then(label::fold)
    else {
        return;
    };
    if streets.contains(&name) || streets.len() >= label::MAX_CANDIDATES {
        return;
    }
    // The anchor rules work in cells of one pixel by two: use the label
    // grid's cells instead.
    let cells: Vec<Point> = points
        .iter()
        .map(|&(x, y)| (x / cell.0, 2.0 * y / cell.1))
        .collect();
    if let Some((col, row)) = label::street_anchor(&cells, name.chars().count(), cols, rows) {
        streets.insert(name.clone());
        let p = (f64::from(col) * cell.0, f64::from(row) * cell.1);
        labels.push((
            Candidate {
                priority: STREET_NAMES.priority,
                col,
                row,
                marker: None,
                text: name,
                colour: STREET_NAME_COLOUR,
                keep_marker: false,
            },
            p,
        ));
    }
}

/// Point in any of `rings` (even-odd over all of them).
fn inside(rings: &[Vec<Point>], p: Point) -> bool {
    let mut c = false;
    for r in rings {
        let n = r.len();
        for i in 0..n {
            let (a, b) = (r[i], r[(i + n - 1) % n]);
            if (a.1 > p.1) != (b.1 > p.1) && p.0 < (b.0 - a.0) * (p.1 - a.1) / (b.1 - a.1) + a.0 {
                c = !c;
            }
        }
    }
    c
}

/// The coverage edge's colour and pattern (as in the terminal).
pub const EDGE: (Colour, LinePattern) = COVERAGE_EDGE;

#[cfg(test)]
mod tests {
    use super::*;
    use resilience_core::maps::geo::LatLon;

    #[test]
    fn empty_view_has_sea_with_an_overview_only() {
        let v = View::new(LatLon::wrapped(49.6, 6.1), 1000.0);
        let (c, r) = plan_size(801, 601);
        let s = scene(&v, c, r, &[], LABEL_CELL);
        assert_eq!(s.background, Colour::Black);
        assert!(s.shapes.is_empty() && s.labels.is_empty());
        assert_eq!((s.width, s.height), (801, 602));
        assert!(s.veil.is_none(), "coarse scale: no veil");
    }

    #[test]
    fn plan_size_is_bounded() {
        let max = resilience_core::maps::geo::MAX_VIEW_PIXELS as u16;
        assert_eq!(plan_size(5000, 5000), (max - 1, max / 2 - 1));
        assert_eq!(plan_size(0, 0), (1, 1));
        for (w, h) in [(5000, 5000), (1023, 1023), (1, 1), (640, 480)] {
            let (c, r) = plan_size(w, h);
            let v = View::new(LatLon::wrapped(0.0, 0.0), 1000.0);
            let vp = render::viewport(&v, c, r);
            assert_eq!(vp.width(), 2 * u32::from(c / 2) + 1);
            assert_eq!(vp.height(), 2 * (2 * u32::from(r / 2) + 1));
        }
    }

    #[test]
    fn inside_even_odd() {
        let sq = vec![vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]];
        assert!(inside(&sq, (5.0, 5.0)));
        assert!(!inside(&sq, (15.0, 5.0)));
        let mut holed = sq.clone();
        holed.push(vec![(2.0, 2.0), (8.0, 2.0), (8.0, 8.0), (2.0, 8.0)]);
        assert!(!inside(&holed, (5.0, 5.0)));
        assert!(inside(&holed, (1.0, 5.0)));
    }

    #[test]
    fn non_finite_points_never_meet() {
        assert!(!meets(10, 10, &[(f64::NAN, 0.0), (1.0, 1.0)]));
        assert!(meets(10, 10, &[(1.0, 1.0), (2.0, 2.0)]));
    }
}
