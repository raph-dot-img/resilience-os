//! Packs and a view to a canvas (ADR 0016 §3). Pure: no files, no
//! terminal; the viewer gives it decoded tiles and draws the canvas it
//! returns.
//!
//! The recipe of `style.rs`, per source (one open pack), in precedence
//! order (overviews first, then detailed packs, oldest data first so that
//! the newest is drawn last and wins where they overlap):
//!
//! 1. The background: the sea colour when an overview is open (its land
//!    polygons are drawn over it), black (land) otherwise.
//! 2. Each source's features are collected with their style rule, sorted
//!    stably by `sort_key`, and the areas and lines drawn. An overview draws
//!    straight on the map. A detailed pack draws on its own layer, copied
//!    onto the map **inside its area only** (its coverage polygon): at
//!    [`fine`] scales (500 m per pixel or finer) the layer starts black and
//!    replaces everything inside, so the overview's overzoomed coastline and
//!    sea never show there (the pack's own water applies); at coarser
//!    scales only what the pack drew is copied over the overview.
//! 3. Partial coverage: at fine scales, cells outside every detailed area
//!    are shaded; at coarser scales each area's edge is drawn dotted.
//! 4. Labels of every source, except those inside the area of a source
//!    drawn later; placed greedily around the reserved crosshair cell, then
//!    the crosshair.
//!
//! **Dashed lines across tile edges.** A line crossing a tile edge reaches
//! the viewer as two features, one per tile. Starting each dash pattern at
//! 0 would make the dashes jump at every edge, so each line's pattern
//! starts at a phase given by its position: the coordinate of its first
//! vertex along the main axis of its first segment, in pixels of the whole
//! world ([`dash_phase`]). Along a straight line both pieces then use the
//! same pattern pixel for pixel, and the parts drawn twice in the tiles'
//! buffers coincide.
//!
//! **The viewport has odd dimensions** around the crosshair cell
//! `(cols / 2, rows / 2)`, so that the view's centre (the coordinates in
//! the readout) is the centre of that cell, not the corner between four.

use std::collections::HashSet;
use std::sync::Arc;

use resilience_core::maps::canvas::{Canvas, CellMask, Colour, LinePattern, Point};
use resilience_core::maps::geo::{
    LatLon, MAX_TILE_ZOOM, TileId, TileTransform, View, ViewTile, Viewport, ZoomRange,
};
use resilience_core::maps::label::{self, Candidate, CellRect};
use resilience_core::maps::mvt::{self, GeomType, PartRole};
use resilience_core::maps::style::{
    self, COVERAGE_EDGE, CROSSHAIR, Class, Draw, Layer, NO_DETAIL_MAX_SCALE, NO_DETAIL_SHADE, Rule,
    SEA, STREET_NAME_COLOUR, STREET_NAMES,
};

use crate::area::Area;

/// A pack whose highest zoom is at most this is an overview: its data is
/// never detailed enough at fine scales, so it never counts as covering a
/// place in detail (whatever its coverage table says).
pub const OVERVIEW_MAX_ZOOM: u8 = 8;

/// At most this many tiles are read for one view, over every pack.
pub const MAX_PLANNED_TILES: usize = 64;

/// What a pack is for drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The world overview kind: drawn first, everywhere.
    Overview,
    /// Has a coverage polygon: drawn inside it, lifts the shading there.
    Detailed,
    /// No coverage polygon: drawn inside its bounds rectangle, which is
    /// treated as an outer limit only (see `app.rs`).
    Approximate,
}

/// 500 m per pixel or finer: shading outside detailed maps, and detailed
/// maps replace the overview inside their area.
pub fn fine(scale: f64) -> bool {
    scale <= NO_DETAIL_MAX_SCALE * (1.0 + 1e-6)
}

/// The viewport for a map of `cols` x `rows` cells: odd sizes centred on
/// the crosshair cell (see the module documentation).
pub fn viewport(view: &View, cols: u16, rows: u16) -> Viewport {
    let w = 2 * u32::from(cols / 2) + 1;
    let h = 2 * (2 * u32::from(rows / 2) + 1);
    view.viewport(w, h)
}

/// The crosshair cell.
pub fn crosshair(cols: u16, rows: u16) -> (u16, u16) {
    (cols / 2, rows / 2)
}

/// The tiles one pack contributes to a view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wanted {
    /// The view is coarser than the pack's minimum zoom: nothing is drawn
    /// from it and nothing is read.
    Underzoom,
    /// The pack's area is not in view.
    Outside,
    /// More tiles than [`geo::MAX_VIEW_TILES`](resilience_core::maps::geo::MAX_VIEW_TILES).
    TooMany,
    Tiles(Vec<ViewTile>),
}

/// The view's rectangle in world units: (x0, y0, x1, y1).
fn world_rect(vp: &Viewport) -> (f64, f64, f64, f64) {
    let t = world_transform(vp);
    let x0 = -t.offset_x / t.scale;
    let y0 = -t.offset_y / t.scale;
    let x1 = x0 + f64::from(vp.width()) / t.scale;
    let y1 = y0 + f64::from(vp.height()) / t.scale;
    (x0, y0, x1, y1)
}

/// World units to view pixels: `px = offset_x + x * scale`.
fn world_transform(vp: &Viewport) -> TileTransform {
    let root = ViewTile {
        // The one tile of zoom 0 is the whole world.
        id: TileId::containing(LatLon::wrapped(0.0, 0.0), 0),
        wrap: 0,
    };
    vp.tile_transform(root, 1)
}

/// The tiles of a pack holding `zoom` with data inside `area` (all of the
/// world for `None`) that the view needs.
pub fn wanted(vp: &Viewport, zoom: ZoomRange, area: Option<&Area>) -> Wanted {
    let ideal = ZoomRange::new(0, MAX_TILE_ZOOM).map_or(0, |all| vp.tile_zoom(all));
    if ideal < zoom.min() {
        return Wanted::Underzoom;
    }
    let (x0, y0, x1, y1) = world_rect(vp);
    if let Some(a) = area
        && ![-1.0, 0.0, 1.0].iter().any(|k| a.meets(*k, x0, y0, x1, y1))
    {
        return Wanted::Outside;
    }
    // Above the pack's highest zoom its tiles are enlarged (overzoom).
    let z = ideal.min(zoom.max());
    let Ok(tiles) = vp.tiles(z) else {
        return Wanted::TooMany;
    };
    let n = f64::from(1u32 << z);
    let tiles: Vec<ViewTile> = tiles
        .into_iter()
        .filter(|t| {
            let Some(a) = area else { return true };
            let tx0 = f64::from(t.id.x()) / n;
            let ty0 = f64::from(t.id.y()) / n;
            [-1.0, 0.0, 1.0]
                .iter()
                .any(|k| a.meets(*k, tx0, ty0, tx0 + 1.0 / n, ty0 + 1.0 / n))
        })
        .collect();
    if tiles.is_empty() {
        Wanted::Outside
    } else {
        Wanted::Tiles(tiles)
    }
}

/// One open pack, ready to draw.
pub struct Source<'a> {
    pub role: Role,
    /// Where its data is drawn (`None`: everywhere).
    pub area: Option<&'a Area>,
    /// Decoded tiles in view (those still loading are simply missing).
    pub tiles: Vec<(ViewTile, Arc<mvt::Tile>)>,
}

/// The pattern phase of a polyline in pixels of the whole world, so that
/// pieces of one line continue each other (module documentation).
/// `origin` is the world-pixel position of the canvas origin, reduced
/// modulo 15 (a multiple of every pattern period).
pub fn dash_phase(points: &[Point], origin: (f64, f64)) -> f64 {
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        if !(dx.is_finite() && dy.is_finite()) || (dx == 0.0 && dy == 0.0) {
            continue;
        }
        return if dx.abs() >= dy.abs() {
            dx.signum() * (a.0 + origin.0)
        } else {
            dy.signum() * (a.1 + origin.1)
        };
    }
    0.0
}

/// The canvas origin in world pixels, rounded down to a whole pixel (so
/// that the pattern index of a pixel does not depend on where in its pixel
/// a piece of line starts) and reduced modulo 15.
pub(crate) fn pattern_origin(vp: &Viewport) -> (f64, f64) {
    let t = world_transform(vp);
    (
        (-t.offset_x).floor().rem_euclid(15.0),
        (-t.offset_y).floor().rem_euclid(15.0),
    )
}

pub(crate) struct Item<'t> {
    pub(crate) rule: &'static Rule,
    pub(crate) feature: mvt::Feature<'t>,
    pub(crate) transform: TileTransform,
    pub(crate) road: bool,
}

/// The class value of a feature for its layer.
fn rule_of(
    layer: Layer,
    layer_name: &str,
    f: &mvt::Feature<'_>,
    key: Option<u32>,
    scale: f64,
) -> Option<&'static Rule> {
    let v = f.attr_at(key?)?;
    if layer == Layer::Boundaries {
        let level = v
            .as_i64()
            .or_else(|| v.as_str().and_then(|s| s.trim().parse::<i64>().ok()))?;
        return style::lookup_boundary(level, scale);
    }
    style::lookup(layer_name, v.as_str()?, scale)
}

/// Collect a source's drawable features, sorted for drawing.
pub(crate) fn collect<'t>(
    tiles: &'t [(ViewTile, Arc<mvt::Tile>)],
    vp: &Viewport,
    scale: f64,
) -> Vec<Item<'t>> {
    let mut items = Vec::new();
    for (vt, tile) in tiles {
        for layer in tile.layers() {
            let Some(kind) = Layer::parse(layer.name()) else {
                continue;
            };
            let key = layer.key_index(Class::attribute(kind));
            let transform = vp.tile_transform(*vt, layer.extent());
            for f in layer.features() {
                if let Some(rule) = rule_of(kind, layer.name(), &f, key, scale) {
                    items.push(Item {
                        rule,
                        feature: f,
                        transform,
                        road: kind == Layer::Roads,
                    });
                }
            }
        }
    }
    items.sort_by_key(|i| i.rule.sort_key());
    items
}

/// Label text of a point feature, folded; may be empty.
pub(crate) fn label_text(l: &style::LabelRule, f: &mvt::Feature<'_>, scale: f64) -> String {
    if !l.name_visible(scale) {
        return String::new();
    }
    let name = f
        .attr("name")
        .and_then(|v| v.as_str())
        .and_then(label::fold);
    let detail = if l.detail_visible(scale) {
        f.attr("ele")
            .and_then(|v| v.as_f64())
            .filter(|e| e.is_finite() && e.abs() < 10_000.0)
            .map(|e| format!("{} m", e.round() as i64))
    } else {
        None
    };
    match (name, detail) {
        (Some(n), Some(d)) => label::fold(&format!("{n} {d}")).unwrap_or(n),
        (Some(n), None) => n,
        (None, Some(d)) => d,
        (None, None) => String::new(),
    }
}

struct Drawer {
    scale: f64,
    origin: (f64, f64),
    cols: u16,
    rows: u16,
    points: Vec<Point>,
    rings: Vec<Vec<Point>>,
}

impl Drawer {
    /// Draw areas and lines of `items` on `canvas`; push label candidates.
    fn draw(
        &mut self,
        canvas: &mut Canvas,
        items: &[Item<'_>],
        labels: &mut Vec<Candidate>,
        streets: &mut HashSet<String>,
    ) {
        let street_names = STREET_NAMES.name_visible(self.scale);
        for it in items {
            let f = &it.feature;
            let t = it.transform;
            match it.rule.draw {
                Draw::Fill => {
                    if f.geom_type() != GeomType::Polygon {
                        continue;
                    }
                    let mut used = 0;
                    for part in f.parts() {
                        if !matches!(part.role, PartRole::Exterior | PartRole::Interior) {
                            continue;
                        }
                        if self.rings.len() <= used {
                            self.rings.push(Vec::new());
                        }
                        let r = &mut self.rings[used];
                        r.clear();
                        r.extend(part.points.iter().map(|p| t.apply(p.x, p.y)));
                        used += 1;
                    }
                    // Most features of a tile are off screen: skip them
                    // before the scanline fill.
                    if used > 0 && self.rings[..used].iter().any(|r| self.meets(r)) {
                        canvas.fill_polygon(&self.rings[..used], it.rule.colour);
                    }
                }
                Draw::Line(pattern) => {
                    if f.geom_type() != GeomType::LineString {
                        continue;
                    }
                    for part in f.parts() {
                        self.points.clear();
                        self.points
                            .extend(part.points.iter().map(|p| t.apply(p.x, p.y)));
                        if !self.meets(&self.points) {
                            continue;
                        }
                        let phase = if pattern == LinePattern::Solid {
                            0.0
                        } else {
                            dash_phase(&self.points, self.origin)
                        };
                        canvas.polyline_from(&self.points, it.rule.colour, pattern, phase);
                        if it.road && street_names {
                            self.street_label(f, labels, streets);
                        }
                    }
                }
                Draw::Label(l) => {
                    if f.geom_type() != GeomType::Point {
                        continue;
                    }
                    // Folded only for points on screen.
                    let mut text: Option<String> = None;
                    for part in f.parts() {
                        for p in part.points {
                            let (x, y) = t.apply(p.x, p.y);
                            if !(x >= 0.0
                                && y >= 0.0
                                && x < f64::from(self.cols)
                                && y < 2.0 * f64::from(self.rows))
                            {
                                continue;
                            }
                            let text = text.get_or_insert_with(|| label_text(&l, f, self.scale));
                            labels.push(Candidate {
                                priority: l.priority,
                                col: x.floor() as i32,
                                row: (y / 2.0).floor() as i32,
                                marker: l.marker,
                                text: text.clone(),
                                colour: it.rule.colour,
                                keep_marker: l.keep_marker,
                            });
                        }
                    }
                }
            }
        }
    }

    /// Whether the bounding box of `points` meets the canvas (one pixel
    /// of margin). Non-finite points count as meeting it: the canvas
    /// skips them itself.
    fn meets(&self, points: &[Point]) -> bool {
        let (w, h) = (f64::from(self.cols), 2.0 * f64::from(self.rows));
        let (mut x0, mut y0, mut x1, mut y1) = (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        for &(x, y) in points {
            if !(x.is_finite() && y.is_finite()) {
                return true;
            }
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        x1 >= -1.0 && y1 >= -1.0 && x0 <= w + 1.0 && y0 <= h + 1.0
    }

    fn street_label(
        &self,
        f: &mvt::Feature<'_>,
        labels: &mut Vec<Candidate>,
        streets: &mut HashSet<String>,
    ) {
        let Some(raw) = f.attr("name").and_then(|v| v.as_str()) else {
            return;
        };
        let Some(name) = label::fold(raw) else {
            return;
        };
        if streets.contains(&name) || streets.len() >= label::MAX_CANDIDATES {
            return;
        }
        if let Some((col, row)) =
            label::street_anchor(&self.points, name.chars().count(), self.cols, self.rows)
        {
            streets.insert(name.clone());
            labels.push(Candidate {
                priority: STREET_NAMES.priority,
                col,
                row,
                marker: None,
                text: name,
                colour: STREET_NAME_COLOUR,
                keep_marker: false,
            });
        }
    }
}

/// An area's rings in view pixels, once per copy of the world in view.
pub(crate) fn area_pixels(area: &Area, vp: &Viewport) -> Vec<Vec<Point>> {
    let t = world_transform(vp);
    let (x0, y0, x1, y1) = world_rect(vp);
    let mut out = Vec::new();
    for k in [-1.0, 0.0, 1.0] {
        if !area.meets(k, x0, y0, x1, y1) {
            continue;
        }
        for ring in &area.rings {
            out.push(
                ring.iter()
                    .map(|w| (t.offset_x + (w.x + k) * t.scale, t.offset_y + w.y * t.scale))
                    .collect(),
            );
        }
    }
    out
}

/// Draw the map of `cols` x `rows` cells.
pub fn render(view: &View, cols: u16, rows: u16, sources: &[Source<'_>]) -> Canvas {
    let vp = viewport(view, cols, rows);
    let scale = view.scale();
    let fine = fine(scale);
    let mut map = Canvas::new(cols, rows);
    let (w, h) = (map.width(), map.height());
    let has_overview = sources.iter().any(|s| s.role == Role::Overview);
    map.fill(if has_overview { SEA } else { Colour::Black });

    let mut drawer = Drawer {
        scale,
        origin: pattern_origin(&vp),
        cols: map.cols() as u16,
        rows: map.rows() as u16,
        points: Vec::new(),
        rings: Vec::new(),
    };
    // Per source: its label candidates and its pixel mask (None: everywhere).
    let mut labels: Vec<Vec<Candidate>> = Vec::with_capacity(sources.len());
    let mut masks: Vec<Option<Canvas>> = Vec::with_capacity(sources.len());
    let mut streets = HashSet::new();
    let mut edges: Vec<Vec<Point>> = Vec::new();
    let mut shade = CellMask::new(cols, rows);

    for s in sources {
        let items = collect(&s.tiles, &vp, scale);
        let mut own = Vec::new();
        match (s.role, s.area) {
            (Role::Overview, _) | (_, None) => {
                drawer.draw(&mut map, &items, &mut own, &mut streets);
                masks.push(None);
            }
            (role, Some(area)) => {
                let rings = area_pixels(area, &vp);
                let mut mask = Canvas::new(cols, rows);
                mask.fill_polygon(&rings, Colour::White);
                shade.add_polygon(&rings);
                if !fine || role == Role::Approximate {
                    for r in &rings {
                        let mut closed = r.clone();
                        if let Some(first) = r.first() {
                            closed.push(*first);
                        }
                        edges.push(closed);
                    }
                }
                let mut layer = Canvas::new(cols, rows);
                if fine {
                    layer.fill(Colour::Black);
                }
                drawer.draw(&mut layer, &items, &mut own, &mut streets);
                for y in 0..h {
                    for x in 0..w {
                        if mask.pixel(x, y).is_none() {
                            continue;
                        }
                        if let Some(c) = layer.pixel(x, y) {
                            map.set_point((x as f64 + 0.5, y as f64 + 0.5), c);
                        }
                    }
                }
                own.retain(|c| inside(&mask, c));
                masks.push(Some(mask));
            }
        }
        labels.push(own);
    }

    if fine {
        shade.invert();
        map.shade(&shade, NO_DETAIL_SHADE);
    }
    for e in &edges {
        map.polyline(e, COVERAGE_EDGE.0, COVERAGE_EDGE.1);
    }

    // Labels: a source's labels inside the area of a source drawn later
    // (on top) are left out, so a place is not named twice.
    let mut all = Vec::new();
    for (i, own) in labels.into_iter().enumerate() {
        for c in own {
            let covered = masks[i + 1..].iter().zip(&sources[i + 1..]).any(|(m, s)| {
                s.role != Role::Overview && m.as_ref().is_some_and(|m| inside(m, &c))
            });
            if !covered {
                all.push(c);
            }
        }
    }
    let (cc, cr) = crosshair(cols, rows);
    let reserved = [CellRect {
        col: cc,
        row: cr,
        width: 1,
        height: 1,
    }];
    let placed = label::place(&all, cols, rows, &reserved);
    label::draw(&mut map, &placed);
    map.put_char(i64::from(cc), i64::from(cr), CROSSHAIR, None, None, true);
    map
}

fn inside(mask: &Canvas, c: &Candidate) -> bool {
    c.col >= 0 && c.row >= 0 && mask.pixel(c.col as usize, 2 * c.row as usize + 1).is_some()
}
