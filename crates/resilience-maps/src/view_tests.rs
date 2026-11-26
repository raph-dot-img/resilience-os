//! Viewer tests (ADR 0016 §8, "TestBackend screen tests (viewer)" and the
//! renderer): synthetic packs written at test time, the reader driven
//! synchronously ([`Worker::handle_all`]), screens drawn with ratatui's
//! `TestBackend` and compared as text.
//!
//! `RESILIENCE_DUMP=1 cargo test -p resilience-maps dump_viewer_screens --
//! --nocapture` prints the screens.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use resilience_core::config::ThemeName;
use resilience_core::maps::canvas::{Canvas, Colour, LinePattern, is_console_char};
use resilience_core::maps::geo::{LatLon, TileId, View, ViewTile, ZoomRange};
use resilience_core::maps::mvt;

use crate::app::{Effect, MapApp, SLOW_AFTER, Screen, Start};
use crate::area::Area;
use crate::cache::Key;
use crate::fixtures::{Attr, PackSpec, TempDir, TileMaker, tile_xy};
use crate::loader::{Catalog, Integrity, Request, Response, TileResult, Worker};
use crate::render::{self, Role, Source, Wanted};
use crate::ui;

// ---------------------------------------------------------------------------
// Test land
// ---------------------------------------------------------------------------

/// Testland: a detailed pack around Testville.
const TESTVILLE: (f64, f64) = (49.6116, 6.1319);
const TESTLAND: (f64, f64, f64, f64) = (6.0, 49.5, 6.3, 49.7);

/// Every tile of zoom `z` meeting the box (w, s, e, n).
fn tiles_over(z: u8, b: (f64, f64, f64, f64)) -> Vec<TileId> {
    let nw = TileId::containing(LatLon::wrapped(b.3, b.0), z);
    let se = TileId::containing(LatLon::wrapped(b.1, b.2), z);
    let mut out = Vec::new();
    for y in nw.y()..=se.y() {
        for x in nw.x()..=se.x() {
            out.extend(TileId::new(z, x, y));
        }
    }
    out
}

/// The same features in every tile of zooms `zooms` over the box.
fn spread(
    mut spec: PackSpec,
    zooms: std::ops::RangeInclusive<u8>,
    b: (f64, f64, f64, f64),
    draw: impl Fn(&mut TileMaker),
) -> PackSpec {
    for z in zooms {
        for id in tiles_over(z, b) {
            let mut t = TileMaker::new(id);
            draw(&mut t);
            spec = spec.tile(t);
        }
    }
    spec
}

fn testland_features(t: &mut TileMaker) {
    t.polygon(
        "water",
        &[("class", Attr::S("lake"))],
        &[(49.64, 6.20), (49.64, 6.24), (49.66, 6.24), (49.66, 6.20)],
    );
    // A vertical primary road through Testville's longitude.
    t.line(
        "roads",
        &[
            ("class", Attr::S("primary")),
            ("name", Attr::S("Route du Test")),
        ],
        &[(49.52, TESTVILLE.1), (49.69, TESTVILLE.1)],
    );
    t.line(
        "roads",
        &[
            ("class", Attr::S("residential")),
            ("name", Attr::S("Rue de l'Essai")),
        ],
        &[(49.6100, 6.10), (49.6100, 6.16)],
    );
    t.line(
        "boundaries",
        &[("admin_level", Attr::I(2))],
        &[(49.52, 6.0), (49.52, 6.3)],
    );
    t.point(
        "places",
        &[("class", Attr::S("town")), ("name", Attr::S("Testville"))],
        (49.6300, 6.1319),
    );
    t.point(
        "pois",
        &[
            ("category", Attr::S("health")),
            ("name", Attr::S("Pharmacie")),
        ],
        (49.6120, 6.1340),
    );
}

fn testland(name: &str, date: &str) -> PackSpec {
    let mut spec = PackSpec::detailed(name, TESTLAND);
    spec.data_date = date.into();
    spec.center = Some((TESTVILLE.1, TESTVILLE.0, 12));
    spread(spec, 0..=13, TESTLAND, testland_features)
}

/// The overview: land over a box around Testland, one city, one border.
const WORLD_LAND: (f64, f64, f64, f64) = (2.0, 47.0, 10.0, 52.0);

fn world() -> PackSpec {
    let mut spec = PackSpec::overview("World overview");
    spec.center = Some((5.0, 49.0, 4));
    spread(spec, 0..=7, WORLD_LAND, |t| {
        t.polygon(
            "land",
            &[("class", Attr::S("landmass"))],
            &[(47.0, 2.0), (47.0, 10.0), (52.0, 10.0), (52.0, 2.0)],
        );
        t.point(
            "places",
            &[("class", Attr::S("city")), ("name", Attr::S("Bigcity"))],
            (48.5, 4.0),
        );
        t.line(
            "boundaries",
            &[("admin_level", Attr::I(2))],
            &[(49.4, 2.0), (49.4, 10.0)],
        );
    })
}

struct Harness {
    _dir: TempDir,
    app: MapApp,
    worker: Worker,
    now: Instant,
}

impl Harness {
    fn new(
        tag: &str,
        packs: &[(&str, &PackSpec)],
        at: Option<(f64, f64)>,
        scale: Option<f64>,
    ) -> Harness {
        Harness::with(tag, packs, at, scale, None, Catalog::none())
    }

    fn with(
        tag: &str,
        packs: &[(&str, &PackSpec)],
        at: Option<(f64, f64)>,
        scale: Option<f64>,
        saved: Option<View>,
        catalog: Catalog,
    ) -> Harness {
        let dir = TempDir::new(tag);
        let mut paths = Vec::new();
        for (name, spec) in packs {
            let p = dir.file(name);
            spec.write(&p);
            paths.push(p);
        }
        Harness::on_files(dir, paths, at, scale, saved, catalog)
    }

    fn on_files(
        dir: TempDir,
        paths: Vec<PathBuf>,
        at: Option<(f64, f64)>,
        scale: Option<f64>,
        saved: Option<View>,
        catalog: Catalog,
    ) -> Harness {
        let now = Instant::now();
        let app = MapApp::new(Start {
            packs: paths,
            at: at.map(|(a, o)| LatLon::new(a, o).expect("valid")),
            scale,
            search: None,
            saved,
            theme: ThemeName::Green,
            now,
        });
        let mut h = Harness {
            _dir: dir,
            app,
            worker: Worker::new(catalog),
            now,
        };
        h.pump();
        h
    }

    /// Carry requests to the reader and answers back until quiet.
    fn pump(&mut self) {
        for _ in 0..20 {
            let reqs = self.app.take_requests();
            if reqs.is_empty() {
                return;
            }
            for r in self.worker.handle_all(reqs) {
                self.app.on_response(r);
            }
        }
        panic!("the viewer keeps asking");
    }

    /// Draw, answer what the drawing asked for, draw again.
    fn screen(&mut self, w: u16, h: u16) -> String {
        let mut text = render(&mut self.app, w, h);
        for _ in 0..5 {
            if self.app.has_requests() {
                self.pump();
                text = render(&mut self.app, w, h);
            } else {
                break;
            }
        }
        text
    }

    fn canvas(&mut self, cols: u16, rows: u16) -> Canvas {
        let _ = self.app.map_canvas(cols, rows);
        self.pump();
        self.app.map_canvas(cols, rows)
    }

    fn key(&mut self, c: KeyCode) -> Effect {
        self.app.handle_key(KeyEvent::new(c, KeyModifiers::NONE))
    }

    fn shift(&mut self, c: KeyCode) -> Effect {
        self.app.handle_key(KeyEvent::new(c, KeyModifiers::SHIFT))
    }

    fn tick(&mut self, d: Duration) {
        self.now += d;
        self.app.on_tick(self.now);
    }
}

fn render(app: &mut MapApp, w: u16, h: u16) -> String {
    let mut t = Terminal::new(TestBackend::new(w, h)).expect("terminal");
    t.draw(|f| ui::draw(f, app)).expect("draw");
    let buf = t.backend().buffer().clone();
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Like [`render`], but a blank cell with a coloured background shows the
/// colour's letter (b blue, g green, y yellow, r red, m magenta, c cyan,
/// w white; black and none stay blank), so dumps show what the colours do.
fn render_coloured(app: &mut MapApp, w: u16, h: u16) -> String {
    use ratatui::style::Color;
    let mut t = Terminal::new(TestBackend::new(w, h)).expect("terminal");
    t.draw(|f| ui::draw(f, app)).expect("draw");
    let buf = t.backend().buffer().clone();
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| {
                    let c = &buf[(x, y)];
                    let sym = c.symbol();
                    if sym != " " {
                        return sym.to_string();
                    }
                    match c.bg {
                        Color::Blue => "b",
                        Color::Green => "g",
                        Color::Yellow => "y",
                        Color::Red => "r",
                        Color::Magenta => "m",
                        Color::Cyan => "c",
                        Color::Gray => "w",
                        _ => " ",
                    }
                    .to_string()
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

impl Harness {
    fn coloured(&mut self, w: u16, h: u16) -> String {
        let _ = self.screen(w, h);
        render_coloured(&mut self.app, w, h)
    }
}

fn assert_console_glyphs(screen: &str) {
    for ch in screen.chars() {
        assert!(
            ch == '\n' || ch.is_ascii_graphic() || ch == ' ' || is_console_char(ch),
            "glyph {ch:?} (U+{:04X}) is not in the console font\n{screen}",
            ch as u32
        );
    }
}

/// The screen's words on one line, without the frame (for wrapped text).
fn flat(screen: &str) -> String {
    screen
        .replace(['│', '┌', '┐', '└', '┘', '─'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn readout_line(screen: &str) -> String {
    screen
        .lines()
        .find(|l| l.contains(" N  ") || l.contains(" S  "))
        .unwrap_or_else(|| panic!("no readout in\n{screen}"))
        .to_string()
}

/// (lat, lon) shown in the decimal readout.
fn readout(screen: &str) -> (f64, f64) {
    let l = readout_line(screen);
    let mut it = l.split_whitespace();
    let lat: f64 = it.next().and_then(|v| v.parse().ok()).expect("lat");
    let ns = it.next().expect("N/S");
    let lon: f64 = it.next().and_then(|v| v.parse().ok()).expect("lon");
    let ew = it.next().expect("E/W");
    (
        if ns == "S" { -lat } else { lat },
        if ew == "W" { -lon } else { lon },
    )
}

// ---------------------------------------------------------------------------
// Map screen
// ---------------------------------------------------------------------------

#[test]
fn first_screen_on_the_overview_has_readout_scale_bar_and_attribution() {
    let w = world();
    let mut h = Harness::new("first-overview", &[("world.mbtiles", &w)], None, None);
    let s = h.screen(80, 24);
    // The overview's centre (5 E, 49 N) at the scale of its zoom 4.
    assert!(s.contains("49.00000 N  5.00000 E"), "{s}");
    assert!(
        s.contains('└') && s.contains('┘') && s.contains(" km"),
        "{s}"
    );
    assert!(s.contains("Natural Earth"), "{s}");
    assert!(s.contains("Map: World overview"), "{s}");
    assert!(s.contains("q Quit") && s.contains("+/- Zoom"), "{s}");
    assert!(s.contains("Bigcity"), "the overview's city label\n{s}");
    assert_console_glyphs(&s);
}

#[test]
fn first_screen_on_a_detailed_pack_starts_at_its_centre() {
    let (w, t) = (world(), testland("Testland", "2026-01-01"));
    let mut h = Harness::new(
        "first-detailed",
        &[("world.mbtiles", &w), ("testland.mbtiles", &t)],
        None,
        None,
    );
    let s = h.screen(80, 24);
    assert!(s.contains("49.61160 N  6.13190 E"), "{s}");
    // Zoom 12 at 49.6 N is about 25 m per pixel: the 20 m step.
    assert_eq!(h.app.view().scale(), 20.0);
    assert!(s.contains("Map: Testland"), "{s}");
    assert!(s.contains("(c) OpenStreetMap contributors"), "{s}");
    assert!(!s.contains("No detailed map here"), "{s}");
    assert_console_glyphs(&s);
    // At 10 m per pixel the names of points of interest and streets show.
    h.key(KeyCode::Char('+'));
    let s = h.screen(80, 24);
    assert!(
        s.contains("H Pharmacie") && s.contains("Rue de l'Essai"),
        "{s}"
    );
    // Zoomed out, the town's name.
    for _ in 0..3 {
        h.key(KeyCode::Char('-'));
    }
    let s = h.screen(80, 24);
    assert!(s.contains("*Testville"), "{s}");
}

#[test]
fn zoom_and_pan_move_the_readout_and_known_places_land_on_the_crosshair() {
    let t = testland("Testland", "2026-01-01");
    let mut h = Harness::new(
        "zoom-pan",
        &[("t.mbtiles", &t)],
        Some((49.62, TESTVILLE.1)),
        Some(50.0),
    );
    let s = h.screen(80, 24);
    assert_eq!(readout(&s), (49.62, 6.1319));
    // The vertical road along Testville's longitude goes through the
    // crosshair's column, above and below it.
    let (cols, rows) = (80u16, 20u16);
    let (cc, cr) = render::crosshair(cols, rows);
    let c = h.canvas(cols, rows);
    let cx = usize::from(cc);
    for y in [2 * usize::from(cr) - 6, 2 * usize::from(cr) + 6] {
        assert_eq!(
            c.pixel(cx, y),
            Some(Colour::Yellow),
            "row {y}\n{}",
            c.pixel_picture()
        );
        assert_ne!(c.pixel(cx - 2, y), Some(Colour::Yellow));
    }
    // Zoom: the readout stays, the scale steps.
    h.key(KeyCode::Char('+'));
    assert_eq!(h.app.view().scale(), 20.0);
    assert_eq!(readout(&h.screen(80, 24)), (49.62, 6.1319));
    h.key(KeyCode::Char('-'));
    h.key(KeyCode::Char('-'));
    assert_eq!(h.app.view().scale(), 100.0);
    let s = h.screen(80, 24);
    assert!(s.contains("100 per pixel") || s.contains('└'), "{s}");
    // Pan east by a quarter screen: 20 pixels of 100 m.
    h.key(KeyCode::Char('l'));
    let (lat, lon) = readout(&h.screen(80, 24));
    assert!((lat - 49.62).abs() < 1e-4, "{lat}");
    let expected = 6.1319 + 20.0 * 100.0 / (111_319.49 * 49.62f64.to_radians().cos());
    assert!((lon - expected).abs() < 2e-4, "{lon} vs {expected}");
    // The road is now a quarter screen to the left of the crosshair.
    let c = h.canvas(cols, rows);
    let y = 2 * usize::from(cr) + 6;
    let road: Vec<usize> = (0..c.width())
        .filter(|&x| c.pixel(x, y) == Some(Colour::Yellow))
        .collect();
    assert!(
        road.iter()
            .any(|&x| (x as i64 - (cx as i64 - 20)).abs() <= 1),
        "{road:?}"
    );
    // And back.
    h.key(KeyCode::Char('h'));
    assert_eq!(readout(&h.screen(80, 24)), (49.62, 6.1319));
    // North, then a single cell south (two pixels of 100 m).
    h.key(KeyCode::Up);
    let (lat_up, _) = readout(&h.screen(80, 24));
    assert!(lat_up > 49.62);
    h.key(KeyCode::Down);
    h.shift(KeyCode::Down);
    let (lat_cell, _) = readout(&h.screen(80, 24));
    let one_cell = 200.0 / 111_320.0;
    assert!((49.62 - lat_cell - one_cell).abs() < 2e-4, "{lat_cell}");
    // d switches to degrees, minutes and seconds.
    h.key(KeyCode::Char('d'));
    let s = h.screen(80, 24);
    assert!(s.contains("49d3") && s.contains("6d07'"), "{s}");
}

#[test]
fn outside_the_coverage_no_detailed_map_and_shading() {
    let (w, t) = (world(), testland("Testland", "2026-01-01"));
    let mut h = Harness::new(
        "no-detail",
        &[("world.mbtiles", &w), ("t.mbtiles", &t)],
        Some((49.0, 6.13)),
        Some(50.0),
    );
    let s = h.screen(80, 24);
    assert!(
        s.contains("No detailed map here: world overview only"),
        "{s}"
    );
    assert!(s.contains('░'), "{s}");
    // Inside: no shading at the crosshair, no message.
    let mut h2 = Harness::new(
        "no-detail-inside",
        &[("world.mbtiles", &w), ("t.mbtiles", &t)],
        Some(TESTVILLE),
        Some(50.0),
    );
    let s = h2.screen(80, 24);
    assert!(!s.contains("No detailed map here"), "{s}");
    // At 5 km per pixel: no shading, the edge of Testland is dotted.
    h.app
        .handle_key(KeyEvent::new(KeyCode::Char('-'), KeyModifiers::NONE));
    for _ in 0..5 {
        h.key(KeyCode::Char('-'));
    }
    assert_eq!(h.app.view().scale(), 5_000.0);
    let s = h.screen(80, 24);
    assert!(
        !s.contains("No detailed map here") && !s.contains('░'),
        "{s}"
    );
    let c = h.canvas(80, 20);
    let white = (0..c.height())
        .flat_map(|y| (0..c.width()).map(move |x| (x, y)))
        .filter(|&(x, y)| c.pixel(x, y) == Some(Colour::White))
        .count();
    assert!(white >= 4, "the dotted edge\n{}", c.pixel_picture());
    // Only the overview: every cell is shaded at 50 m.
    let mut h3 = Harness::new(
        "only-overview",
        &[("world.mbtiles", &w)],
        Some(TESTVILLE),
        Some(50.0),
    );
    let s = h3.screen(80, 24);
    assert!(s.contains("No detailed map here"), "{s}");
}

#[test]
fn only_detailed_packs_work_without_an_overview() {
    let t = testland("Testland", "2026-01-01");
    let mut h = Harness::new("only-detailed", &[("t.mbtiles", &t)], None, None);
    let s = h.screen(80, 24);
    assert!(
        s.contains("Map: Testland") && !s.contains("No detailed map"),
        "{s}"
    );
    // Outside it, without an overview.
    h.app
        .handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    let mut far = Harness::new(
        "only-detailed-far",
        &[("t.mbtiles", &t)],
        Some((10.0, 10.0)),
        Some(50.0),
    );
    let s = far.screen(80, 24);
    assert!(
        s.contains("No detailed map here: no open map covers this place"),
        "{s}"
    );
    assert!(
        s.contains("No map here") && s.contains("No map data in view"),
        "{s}"
    );
}

#[test]
fn several_packs_overlap_newest_on_top() {
    // A (older) and B (newer) overlap on 6.10-6.20 E.
    let a_box = (6.0, 49.5, 6.2, 49.7);
    let b_box = (6.1, 49.5, 6.3, 49.7);
    let lake = |t: &mut TileMaker| {
        t.polygon(
            "water",
            &[("class", Attr::S("lake"))],
            &[(49.55, 6.02), (49.55, 6.27), (49.65, 6.27), (49.65, 6.02)],
        );
        t.point(
            "places",
            &[("class", Attr::S("town")), ("name", Attr::S("Oldtown"))],
            (49.603, 6.125),
        );
    };
    let mut a = PackSpec::detailed("Alpha", a_box);
    a.data_date = "2025-01-01".into();
    let a = spread(a, 9..=11, a_box, lake);
    let mut b = PackSpec::detailed("Beta", b_box);
    b.data_date = "2026-01-01".into();
    let b = spread(b, 9..=11, b_box, |t| {
        t.point(
            "places",
            &[("class", Attr::S("town")), ("name", Attr::S("Newtown"))],
            (49.597, 6.115),
        );
    });
    // B is given first, A second: the date decides, not the order.
    let mut h = Harness::new(
        "overlap",
        &[("b.mbtiles", &b), ("a.mbtiles", &a)],
        Some((49.60, 6.11)),
        Some(50.0),
    );
    assert_eq!(
        h.app.drawing_order(),
        vec![1, 0],
        "A (older) first, B on top"
    );
    let c = h.canvas(80, 20);
    let (cc, cr) = render::crosshair(80, 20);
    // In the overlap, B (no lake) replaces A: black, not blue.
    let y = 2 * usize::from(cr) + 3;
    assert_eq!(
        c.pixel(usize::from(cc) + 3, y),
        Some(Colour::Black),
        "{}",
        c.pixel_picture()
    );
    // West of B, A's lake shows.
    assert!(
        (0..usize::from(cc) - 10).any(|x| c.pixel(x, y) == Some(Colour::Blue)),
        "{}",
        c.pixel_picture()
    );
    let s = h.screen(80, 24);
    assert!(s.contains("Map: Beta"), "{s}");
    assert!(s.contains("Newtown"), "{s}");
    assert!(
        !s.contains("Oldtown"),
        "A's label inside B is not drawn\n{s}"
    );
    assert!(!s.contains("No detailed map here"), "{s}");
}

#[test]
fn a_pack_without_a_coverage_polygon_is_never_presented_as_precise() {
    let mut t = testland("Thirdparty", "2026-01-01");
    t.coverage = None;
    // Near its western limit (6.0 E), which is in view.
    let mut h = Harness::new(
        "approximate",
        &[("t.mbtiles", &t)],
        Some((49.6, 6.01)),
        Some(50.0),
    );
    let (_, p) = h.app.open_packs().next().expect("open");
    assert_eq!(p.role, Role::Approximate);
    let s = h.screen(100, 24);
    assert!(s.contains("does not say which area it covers"), "{s}");
    // Its outer limit is drawn dotted even at fine scales.
    let c = h.canvas(100, 20);
    assert!(
        (0..c.height()).any(|y| (0..c.width()).any(|x| c.pixel(x, y) == Some(Colour::White))),
        "edge drawn"
    );
    h.key(KeyCode::Char('i'));
    let s = h.screen(100, 40);
    assert!(
        s.contains("not stated by the file: drawn within its outer limits"),
        "{s}"
    );
    // Without bounds either: it never lifts the shading.
    t.bounds = None;
    let mut h = Harness::new(
        "approximate-nobounds",
        &[("t.mbtiles", &t)],
        Some(TESTVILLE),
        Some(50.0),
    );
    let s = h.screen(100, 24);
    assert!(s.contains("No detailed map here"), "{s}");
}

#[test]
fn info_screen_shows_date_coverage_and_checked_state() {
    let t = testland("Testland", "2026-01-01");
    let w = world();
    let dir = TempDir::new("info");
    let packs_dir = dir.file("packs");
    std::fs::create_dir_all(&packs_dir).expect("mkdir");
    let installed = packs_dir.join("world.mbtiles");
    w.write(&installed);
    let stick = dir.file("stick.mbtiles");
    t.write(&stick);
    let cfg = resilience_core::packs::PackConfig::parse(&format!(
        "dir = \"{}\"\nstore_user = \"u\"\nstore = \"/s\"\npkexec = \"/p\"\npkttyagent = \"/a\"\n\
         curl = \"/c\"\nsha256sum = \"/h\"\nkiwix_serve = \"/k\"\nbrowser = [\"/b\"]\n\
         session_shell = \"/sh\"\n",
        packs_dir.display()
    ))
    .expect("config");
    let mut h = Harness::on_files(
        dir,
        vec![installed, stick],
        Some(TESTVILLE),
        Some(50.0),
        None,
        Catalog::from_config(&cfg),
    );
    h.key(KeyCode::Char('i'));
    assert_eq!(h.app.screen, Screen::Info { scroll: 0 });
    let s = h.screen(100, 60);
    assert!(s.contains("MAP INFORMATION"), "{s}");
    assert!(
        s.contains("1. World overview") && s.contains("2. Testland"),
        "{s}"
    );
    assert!(
        s.contains("checked against the catalog of this system"),
        "{s}"
    );
    assert!(
        s.contains("not checked against the catalog"),
        "the stick file\n{s}"
    );
    assert!(flat(&s).contains("passed its integrity check"), "{s}");
    assert!(s.contains("Data date:    2026-01-01"), "{s}");
    assert!(s.contains("Covers:       Testland, whole region"), "{s}");
    assert!(s.contains("Licence:      ODbL-1.0"), "{s}");
    assert!(
        s.contains("Source:       testland.osm.pbf sha256:0123"),
        "{s}"
    );
    assert!(
        s.contains("Attribution:  (c) OpenStreetMap contributors"),
        "{s}"
    );
    assert!(
        flat(&s).contains("OpenStreetMap is made by volunteers. Roads, places and names can be missing, wrong or out of date. Something absent from the map may exist."),
        "{s}"
    );
    assert_console_glyphs(&s);
    // Scrolls at a small size; Esc goes back to the map.
    let small = h.screen(40, 12);
    assert!(small.contains("(1/"), "{small}");
    h.key(KeyCode::End);
    let end = h.screen(40, 12);
    assert!(end.contains("exist."), "{end}");
    h.key(KeyCode::Esc);
    assert_eq!(h.app.screen, Screen::Map);
}

#[test]
fn legend_follows_the_scale_and_shows_all_on_request_and_help() {
    let t = testland("Testland", "2026-01-01");
    let mut h = Harness::new("legend", &[("t.mbtiles", &t)], Some(TESTVILLE), Some(500.0));
    h.key(KeyCode::Char('L'));
    let s = h.screen(80, 40);
    assert!(s.contains("LEGEND AT 500 M PER PIXEL"), "{s}");
    assert!(
        s.contains("Primary road") && s.contains("No detailed map here"),
        "{s}"
    );
    assert!(
        !s.contains("Path, track") && !s.contains("residential"),
        "not drawn at 500 m\n{s}"
    );
    h.key(KeyCode::Char('a'));
    let s = h.screen(120, 60);
    assert!(
        s.contains("LEGEND, ALL SCALES") && s.contains("Path, track"),
        "{s}"
    );
    assert!(s.contains("up to 10 m per pixel"), "{s}");
    assert_console_glyphs(&s);
    h.key(KeyCode::Esc);
    h.key(KeyCode::Char('?'));
    let s = h.screen(80, 40);
    assert!(
        s.contains("MAP HELP") && s.contains("Zoom in and out"),
        "{s}"
    );
    h.key(KeyCode::Esc);
    h.key(KeyCode::F(1));
    assert_eq!(h.app.screen, Screen::Help { scroll: 0 });
    h.key(KeyCode::Char('q'));
    assert_eq!(h.app.screen, Screen::Map);
    assert_eq!(h.key(KeyCode::Char('q')), Effect::Quit);
}

/// Real packs (2026-11-20): an unnamed point is "(no name)", not "(cannot
/// be shown)", and a bare `yes` class (shop=yes) shows no kind.
#[test]
fn nearby_names_unnamed_points_plainly() {
    let t = testland("Testland", "2026-01-01")
        .place("poi", "defibrillator", "", None, (49.6121, 6.1319), 3)
        .place("poi", "yes", "uewen&feier", None, (49.6143, 6.1319), 3);
    let mut h = Harness::new("unnamed", &[("t.mbtiles", &t)], Some(TESTVILLE), Some(50.0));
    h.key(KeyCode::Char('n'));
    let s = h.screen(110, 30);
    assert!(s.contains("(no name) (defibrillator)"), "{s}");
    assert!(!s.contains("cannot be shown"), "{s}");
    assert!(s.contains("uewen&feier  ") && !s.contains("(yes)"), "{s}");
}

/// Nearby (`n`, slice 11): points around the crosshair, nearest first,
/// one kind at a time with Tab, Enter centres the map; peaks count,
/// settlements and streets do not.
#[test]
fn nearby_lists_points_around_the_crosshair() {
    let t = testland("Testland", "2026-01-01")
        .place(
            "poi",
            "pharmacy",
            "Pharmacie du Test",
            Some("Testville"),
            (49.6121, 6.1319),
            3,
        )
        .place(
            "poi",
            "drinking_water",
            "Fontaine",
            None,
            (49.6143, 6.1319),
            1,
        )
        .place("natural", "peak", "Mont Test", None, (49.6197, 6.1319), 2)
        .place("poi", "shelter", "Abri", None, (49.6250, 6.1319), 2)
        .place("poi", "cafe", "Cafe Loin", None, (49.6800, 6.1319), 9)
        .place("settlement", "town", "Testville", None, TESTVILLE, 1)
        .place(
            "street",
            "residential",
            "Rue de l'Essai",
            None,
            (49.6117, 6.1319),
            1,
        );
    let mut h = Harness::new("nearby", &[("t.mbtiles", &t)], Some(TESTVILLE), Some(50.0));
    assert_eq!(h.key(KeyCode::Char('n')), Effect::None);
    assert_eq!(h.app.screen, Screen::Nearby);
    let s = h.screen(110, 30);
    let order: Vec<usize> = [
        "Pharmacie du Test",
        "Fontaine",
        "Mont Test",
        "Abri",
        "Cafe Loin",
    ]
    .iter()
    .map(|n| s.find(n).unwrap_or_else(|| panic!("{n}: {s}")))
    .collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{s}");
    assert!(
        s.contains("health") && s.contains("(pharmacy)") && s.contains("m N"),
        "{s}"
    );
    assert!(
        !s.contains("Rue de l'Essai") && !s.contains("Testville (town)"),
        "{s}"
    );
    assert!(s.contains("Nearest first, from: Testland"), "{s}");
    assert_console_glyphs(&s);
    // Tab: one kind at a time, most urgent first; Shift-Tab back.
    h.key(KeyCode::Tab);
    let s = h.screen(110, 30);
    assert!(
        s.contains("only health") && s.contains("Pharmacie") && !s.contains("Fontaine"),
        "{s}"
    );
    h.key(KeyCode::Tab);
    let s = h.screen(110, 30);
    assert!(
        s.contains("only water") && s.contains("Fontaine") && !s.contains("Pharmacie"),
        "{s}"
    );
    h.key(KeyCode::BackTab);
    h.key(KeyCode::BackTab);
    assert!(h.screen(110, 30).contains("every kind"));
    // Enter shows the chosen point on the map.
    h.key(KeyCode::Down);
    h.key(KeyCode::Enter);
    assert_eq!(h.app.screen, Screen::Map);
    assert!((h.app.view().center().lat() - 49.6143).abs() < 1e-6);
    // From somewhere with nothing near: said plainly.
    h.app
        .handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    type_text(&mut h, "49.52 6.29");
    h.key(KeyCode::Enter);
    h.key(KeyCode::Char('n'));
    assert!(flat(&h.screen(110, 30)).contains("Nothing within 10 km in: Testland"));
}

/// Go to (`g`): typed coordinates in the forms the readout prints;
/// mistakes are explained and nothing moves.
#[test]
fn go_to_coordinates() {
    let t = testland("Testland", "2026-01-01");
    let mut h = Harness::new("goto", &[("t.mbtiles", &t)], Some(TESTVILLE), Some(50.0));
    h.key(KeyCode::Char('g'));
    assert_eq!(h.app.screen, Screen::GoTo);
    let s = h.screen(100, 30);
    assert!(
        s.contains("GO TO COORDINATES") && s.contains("49.6116 6.1319"),
        "{s}"
    );
    let before = *h.app.view();
    type_text(&mut h, "north of here");
    h.key(KeyCode::Enter);
    assert_eq!(h.app.screen, Screen::GoTo);
    assert!(h.screen(100, 30).contains("[!!]"));
    assert_eq!(*h.app.view(), before);
    h.key(KeyCode::Esc);
    h.key(KeyCode::Char('g'));
    for _ in 0..13 {
        h.key(KeyCode::Backspace);
    }
    type_text(&mut h, "49 36 41.8 N 6 7 54.8 E");
    h.key(KeyCode::Enter);
    assert_eq!(h.app.screen, Screen::Map);
    let c = h.app.view().center();
    assert!(
        (c.lat() - 49.611_61).abs() < 1e-4 && (c.lon() - 6.131_89).abs() < 1e-4,
        "{c:?}"
    );
    assert_eq!(h.app.view().scale(), before.scale(), "the scale is kept");
    // A latitude out of range is refused.
    h.key(KeyCode::Char('g'));
    for _ in 0..30 {
        h.key(KeyCode::Backspace);
    }
    type_text(&mut h, "95 6");
    h.key(KeyCode::Enter);
    assert_eq!(h.app.screen, Screen::GoTo);
}

fn type_text(h: &mut Harness, text: &str) {
    for c in text.chars() {
        h.key(KeyCode::Char(c));
    }
}

/// Search (ADR 0016 slice 10): `/`, words, Enter, results from every
/// open pack ranked, Enter centres the map; nothing found says which maps
/// were searched; `--search` runs once the packs are open.
#[test]
fn search_finds_places_and_centres_the_map() {
    let t = testland("Testland", "2026-01-01")
        .place("settlement", "town", "Testville", None, TESTVILLE, 1)
        .place(
            "poi",
            "pharmacy",
            "Pharmacie du Test",
            Some("Testville"),
            (49.6110, 6.1320),
            1,
        )
        .place(
            "street",
            "residential",
            "Rue de l'Essai",
            Some("Testville"),
            (49.6120, 6.1310),
            1,
        )
        .place("settlement", "village", "Farbourg", None, (49.69, 6.29), 3);
    let w = world().place("settlement", "city", "Bigcity", None, (48.5, 4.0), 1);
    let mut h = Harness::new(
        "search",
        &[("w.mbtiles", &w), ("t.mbtiles", &t)],
        Some((49.0, 5.0)),
        Some(20_000.0),
    );
    assert_eq!(h.key(KeyCode::Char('/')), Effect::None);
    assert_eq!(h.app.screen, Screen::Search);
    let s = h.screen(100, 30);
    assert!(s.contains("FIND A PLACE") && s.contains("Find:"), "{s}");
    type_text(&mut h, "tes");
    h.key(KeyCode::Enter);
    let s = h.screen(100, 30);
    // The town first (settlements before points of interest), with where it is.
    let town = s.find("Testville (town)").unwrap_or_else(|| panic!("{s}"));
    let shop = s
        .find("Pharmacie du Test (pharmacy, near Testville)")
        .unwrap_or_else(|| panic!("{s}"));
    assert!(town < shop, "{s}");
    assert!(s.contains("2 found in: World overview, Testland"), "{s}");
    assert!(s.contains("km NE"), "{s}");
    assert_console_glyphs(&s);
    // Enter goes to the first result.
    h.key(KeyCode::Enter);
    assert_eq!(h.app.screen, Screen::Map);
    let c = h.app.view().center();
    assert!((c.lat() - TESTVILLE.0).abs() < 1e-6 && (c.lon() - TESTVILLE.1).abs() < 1e-6);
    assert!(h.app.view().scale() <= 20.0);
    // `/` again: the same words and results; Down + Enter takes the second.
    h.key(KeyCode::Char('/'));
    assert!(h.screen(100, 30).contains("Testville (town)"));
    h.key(KeyCode::Down);
    h.key(KeyCode::Enter);
    let c = h.app.view().center();
    assert!((c.lat() - 49.6110).abs() < 1e-6, "{c:?}");
    // Changing the words searches again; the overview's city is found too.
    h.key(KeyCode::Char('/'));
    for _ in 0..3 {
        h.key(KeyCode::Backspace);
    }
    type_text(&mut h, "big");
    h.key(KeyCode::Enter);
    assert!(h.screen(100, 30).contains("Bigcity (city)"));
    // Nothing found: which maps were searched, and that others cannot be.
    for _ in 0..3 {
        h.key(KeyCode::Backspace);
    }
    type_text(&mut h, "nowhere");
    h.key(KeyCode::Enter);
    let s = flat(&h.screen(100, 30));
    assert!(
        s.contains("Nothing found in: World overview, Testland"),
        "{s}"
    );
    assert!(s.contains("outside these maps cannot be found"), "{s}");
    // Hostile words are only words.
    for _ in 0..7 {
        h.key(KeyCode::Backspace);
    }
    type_text(&mut h, "\" OR \"x*");
    h.key(KeyCode::Enter);
    assert!(flat(&h.screen(100, 30)).contains("Nothing found"));
    // Esc leaves the search without moving.
    let before = *h.app.view();
    h.key(KeyCode::Esc);
    assert_eq!(h.app.screen, Screen::Map);
    assert_eq!(*h.app.view(), before);
}

/// Accented names (ADR 0016, "Decided while showing accented names"):
/// results show the letters the console font has and fold the others;
/// typing with or without accents finds the same place.
#[test]
fn search_shows_accented_names() {
    let t = testland("Testland", "2026-01-01")
        .place("settlement", "town", "Orléans", None, TESTVILLE, 1)
        .place("settlement", "village", "Île-d'Arz", None, (49.69, 6.29), 1);
    let mut h = Harness::new("accents", &[("t.mbtiles", &t)], None, None);
    for words in ["orleans", "Orléans", "ORLÉANS"] {
        h.key(KeyCode::Char('/'));
        for _ in 0..20 {
            h.key(KeyCode::Backspace);
        }
        type_text(&mut h, words);
        h.key(KeyCode::Enter);
        let s = h.screen(100, 30);
        assert!(s.contains("Orléans (town)"), "{words}: {s}");
        assert_console_glyphs(&s);
        h.key(KeyCode::Esc);
    }
    h.key(KeyCode::Char('/'));
    for _ in 0..20 {
        h.key(KeyCode::Backspace);
    }
    type_text(&mut h, "ile");
    h.key(KeyCode::Enter);
    let s = h.screen(100, 30);
    // Î is not in the font: folded.
    assert!(s.contains("Ile-d'Arz (village)"), "{s}");
    assert_console_glyphs(&s);
}

#[test]
fn search_given_at_start_and_maps_without_search() {
    let t = testland("Testland", "2026-01-01").place(
        "settlement",
        "town",
        "Testville",
        None,
        TESTVILLE,
        1,
    );
    let dir = TempDir::new("search-start");
    let p = dir.file("t.mbtiles");
    t.write(&p);
    let now = Instant::now();
    let mut app = MapApp::new(Start {
        packs: vec![p],
        at: None,
        scale: None,
        search: Some("Testville".into()),
        saved: None,
        theme: ThemeName::Green,
        now,
    });
    let mut worker = Worker::new(Catalog::none());
    for _ in 0..10 {
        let reqs = app.take_requests();
        if reqs.is_empty() {
            break;
        }
        for r in worker.handle_all(reqs) {
            app.on_response(r);
        }
    }
    assert_eq!(app.screen, Screen::Search);
    assert_eq!(app.find.results.len(), 1);
    // A map without search tables: said plainly, no request.
    let plain = testland("Plain", "2026-01-01");
    let mut h = Harness::new(
        "nosearch",
        &[("p.mbtiles", &plain)],
        Some(TESTVILLE),
        Some(50.0),
    );
    h.key(KeyCode::Char('/'));
    type_text(&mut h, "test");
    h.key(KeyCode::Enter);
    assert!(!h.app.has_requests());
    assert!(flat(&h.screen(100, 30)).contains("no open map can be searched"));
}

#[test]
fn too_small_terminal_shows_a_plain_message() {
    let t = testland("Testland", "2026-01-01");
    let mut h = Harness::new("small", &[("t.mbtiles", &t)], Some(TESTVILLE), Some(50.0));
    for (w, hh) in [(39, 12), (40, 11), (10, 3), (1, 1)] {
        let s = h.screen(w, hh);
        assert!(
            s.replace('\n', "").contains("Terminal too small") || w < 20,
            "{s}"
        );
        assert!(!s.contains('▀'), "{s}");
    }
    let s = h.screen(40, 12);
    assert!(s.contains(" N  ") && s.contains("q Quit"), "{s}");
    assert_console_glyphs(&s);
    // Every size up to large ones draws without panicking.
    for w in [40, 41, 57, 80, 81, 170, 250] {
        for hh in [12, 13, 25, 48, 60] {
            assert_console_glyphs(&h.screen(w, hh));
        }
    }
}

#[test]
fn no_pack_at_all_says_how_to_get_one() {
    let mut h = Harness::new("nopack", &[], None, None);
    let s = h.screen(80, 24);
    assert!(s.contains("No map is installed or open."), "{s}");
    assert!(
        flat(&s).contains("Maps & navigation, then Map packs"),
        "{s}"
    );
    assert_eq!(h.app.exit_code(), 0);
    assert_console_glyphs(&s);
    h.key(KeyCode::Char('i'));
    let s = h.screen(80, 24);
    assert!(
        s.contains("No map is open") && s.contains("OpenStreetMap is made by volunteers"),
        "{s}"
    );
}

fn damaged_testland() -> PackSpec {
    // At zoom 12 (20 m per pixel) around Testville: one good tile, the
    // others damaged.
    let mut spec = PackSpec::detailed("Brokenland", TESTLAND);
    let ids = tiles_over(12, TESTLAND);
    let good = TileId::containing(LatLon::wrapped(TESTVILLE.0, TESTVILLE.1), 12);
    for (n, id) in ids.iter().enumerate() {
        if *id == good {
            let mut t = TileMaker::new(*id);
            testland_features(&mut t);
            spec = spec.tile(t);
        } else if n % 2 == 0 {
            spec.raw_tiles
                .push((*id, b"\x1f\x8b\x08\x00garbage".to_vec()));
        } else {
            // Valid gzip, invalid MVT.
            spec.raw_tiles
                .push((*id, crate::inflate::gzip(&[0x1a, 0xff, 0xff, 0x01])));
        }
    }
    spec
}

#[test]
fn corrupt_tiles_show_the_rest_and_one_message_and_are_not_retried() {
    let t = damaged_testland();
    assert!(t.raw_tiles.len() >= 2, "{}", t.raw_tiles.len());
    let mut h = Harness::new("corrupt", &[("t.mbtiles", &t)], Some(TESTVILLE), Some(20.0));
    let s = h.screen(170, 48);
    assert!(s.contains("is damaged and is not shown"), "{s}");
    // The good tile's street and point of interest are drawn.
    assert!(s.contains("▀▀▀▀") && s.contains('H'), "{s}");
    assert!(h.app.cache().bad_count() >= 1);
    // Drawing again asks for nothing: bad tiles are remembered.
    let _ = h.app.map_canvas(170, 45);
    h.key(KeyCode::Char('d'));
    let _ = h.screen(170, 48);
    assert!(!h.app.has_requests());
    // The notice goes away; the viewer carries on.
    h.tick(crate::app::NOTICE_TIME);
    let s = h.screen(170, 48);
    assert!(!s.contains("is damaged"), "{s}");
    assert_eq!(h.app.exit_code(), 0);
}

#[test]
fn refused_files_are_explained_and_others_still_shown() {
    let t = testland("Testland", "2026-01-01");
    let dir = TempDir::new("refused");
    let good = dir.file("good.mbtiles");
    t.write(&good);
    let junk = dir.file("junk.mbtiles");
    std::fs::write(&junk, vec![0x42u8; 8192]).expect("write");
    let missing = dir.file("missing.mbtiles");
    let mut h = Harness::on_files(
        dir,
        vec![junk.clone(), good, missing.clone()],
        Some(TESTVILLE),
        Some(50.0),
        None,
        Catalog::none(),
    );
    let s = h.screen(100, 30);
    assert!(s.contains("Map: Testland"), "{s}");
    assert_eq!(h.app.exit_code(), 0);
    h.key(KeyCode::Char('i'));
    let s = h.screen(100, 60);
    assert!(s.contains("Not shown:    this is not a map pack"), "{s}");
    assert!(s.contains("cannot read the file"), "{s}");
    assert_eq!(h.app.failures().len(), 2);
    // Only refused files: a page that says so, and exit status 2.
    let dir = TempDir::new("refused-all");
    let junk2 = dir.file("junk.mbtiles");
    std::fs::write(&junk2, b"SQLite format 3\0 truncated").expect("write");
    let mut h = Harness::on_files(dir, vec![junk2], None, None, None, Catalog::none());
    let s = h.screen(80, 24);
    assert!(
        s.contains("The map cannot be shown.") && s.contains("junk.mbtiles"),
        "{s}"
    );
    assert!(s.contains("copy it again"), "{s}");
    assert_eq!(h.app.exit_code(), 2);
    assert_console_glyphs(&s);
}

#[test]
fn a_damaged_small_file_fails_its_check_and_is_not_used() {
    let mut h = Harness::new("checkfail", &[], None, None);
    // As the reader would answer for a damaged unchecked file.
    let t = testland("Testland", "2026-01-01");
    let dir = TempDir::new("checkfail-pack");
    let p = dir.file("t.mbtiles");
    t.write(&p);
    let mut w = Worker::new(Catalog::none());
    let opened = w.handle_all([Request::Open {
        index: 0,
        path: p.clone(),
    }]);
    let Some(Response::Opened { summary, .. }) = opened.into_iter().next() else {
        panic!("opened")
    };
    let mut app = MapApp::new(Start {
        packs: vec![p],
        at: None,
        scale: None,
        search: None,
        saved: None,
        theme: ThemeName::Mono,
        now: Instant::now(),
    });
    let _ = app.take_requests();
    app.on_response(Response::Opened { index: 0, summary });
    let s = render(&mut app, 80, 24);
    assert!(s.contains("Checking the file Testland"), "{s}");
    app.on_response(Response::Checked {
        index: 0,
        result: Err("the integrity check failed: page 3 is damaged".into()),
    });
    let s = render(&mut app, 80, 24);
    assert!(s.contains("The map cannot be shown."), "{s}");
    assert!(
        flat(&s).contains("Testland: the integrity check failed: page 3 is damaged"),
        "{s}"
    );
    assert_eq!(app.exit_code(), 2);
    h.pump();
}

#[test]
fn a_large_unchecked_file_is_offered_the_check() {
    let t = testland("Testland", "2026-01-01");
    let dir = TempDir::new("offer");
    let p = dir.file("t.mbtiles");
    t.write(&p);
    let mut w = Worker::new(Catalog::none());
    let out = w.handle_all([Request::Open {
        index: 0,
        path: p.clone(),
    }]);
    let Some(Response::Opened { mut summary, .. }) = out.into_iter().next() else {
        panic!("opened")
    };
    summary.integrity = Integrity::NotChecked;
    let mut app = MapApp::new(Start {
        packs: vec![p],
        at: Some(LatLon::new(TESTVILLE.0, TESTVILLE.1).expect("ll")),
        scale: Some(50.0),
        search: None,
        saved: None,
        theme: ThemeName::Green,
        now: Instant::now(),
    });
    let _ = app.take_requests();
    app.on_response(Response::Opened { index: 0, summary });
    let s = render(&mut app, 100, 24);
    assert!(
        s.contains("Testland was not checked. Press c to check the file"),
        "{s}"
    );
    let _ = app.take_requests();
    app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    let reqs = app.take_requests();
    assert!(
        reqs.iter()
            .any(|r| matches!(r, Request::Check { index: 0 })),
        "{reqs:?}"
    );
    let s = render(&mut app, 100, 24);
    assert!(s.contains("Checking the file Testland"), "{s}");
}

#[test]
fn a_slow_reader_never_blocks_the_viewer() {
    let t = testland("Testland", "2026-01-01");
    let mut h = Harness::new("slow", &[("t.mbtiles", &t)], Some(TESTVILLE), Some(50.0));
    // Draw, but do not answer the tile requests (a reader stuck on a slow
    // stick).
    let _ = render(&mut h.app, 80, 24);
    let reqs = h.app.take_requests();
    assert!(!reqs.is_empty());
    let s = render(&mut h.app, 80, 24);
    assert!(s.contains("Reading the map..."), "{s}");
    h.tick(SLOW_AFTER);
    assert!(h.app.dirty);
    let s = render(&mut h.app, 80, 24);
    assert!(
        s.contains("is slow: a slow USB stick or a damaged file"),
        "{s}"
    );
    // Keys still work: the view moves and the screens open.
    h.key(KeyCode::Char('+'));
    assert_eq!(h.app.view().scale(), 20.0);
    h.key(KeyCode::Char('i'));
    assert!(render(&mut h.app, 80, 24).contains("MAP INFORMATION"));
    h.key(KeyCode::Esc);
    // A read that ran past its budget is reported once.
    let key = Key {
        pack: 0,
        tile: TileId::containing(LatLon::wrapped(TESTVILLE.0, TESTVILLE.1), 11),
    };
    h.app.on_response(Response::Tile {
        generation: 99,
        key,
        result: TileResult::Slow,
        elapsed: Duration::from_millis(200),
    });
    let s = render(&mut h.app, 80, 24);
    assert!(s.contains("is too slow to read here"), "{s}");
    // The reader thread going away is said, not hidden.
    h.app.on_reader_lost();
    let s = render(&mut h.app, 80, 24);
    assert!(s.contains("The map reader stopped"), "{s}");
}

#[test]
fn the_last_position_is_used_unless_overridden() {
    let t = testland("Testland", "2026-01-01");
    let saved = View::new(LatLon::new(49.65, 6.2).expect("ll"), 100.0);
    let h = Harness::with(
        "saved",
        &[("t.mbtiles", &t)],
        None,
        None,
        Some(saved),
        Catalog::none(),
    );
    assert_eq!(*h.app.view(), saved);
    assert_eq!(h.app.view_to_save(), Some(saved));
    // --at replaces the position, the saved scale stays.
    let h = Harness::with(
        "saved-at",
        &[("t.mbtiles", &t)],
        Some(TESTVILLE),
        None,
        Some(saved),
        Catalog::none(),
    );
    assert_eq!(h.app.view().scale(), 100.0);
    assert!((h.app.view().center().lat() - TESTVILLE.0).abs() < 1e-9);
    // --scale replaces the scale only.
    let h = Harness::with(
        "saved-scale",
        &[("t.mbtiles", &t)],
        None,
        Some(5.0),
        Some(saved),
        Catalog::none(),
    );
    assert_eq!(h.app.view().scale(), 5.0);
    assert!((h.app.view().center().lat() - 49.65).abs() < 1e-9);
    // Nothing to save before the start position is known.
    let app = MapApp::new(Start {
        packs: vec![PathBuf::from("/m/x.mbtiles")],
        at: None,
        scale: None,
        search: Some("essai".into()),
        saved: None,
        theme: ThemeName::Green,
        now: Instant::now(),
    });
    assert_eq!(app.view_to_save(), None);
    assert_eq!(app.find.input, "essai");
    assert_eq!(app.screen, Screen::Search);
}

// ---------------------------------------------------------------------------
// Renderer, compared as text
// ---------------------------------------------------------------------------

fn decode(t: &TileMaker) -> Arc<mvt::Tile> {
    Arc::new(mvt::decode(&t.encode()).expect("decodes"))
}

/// A view on the equator at `scale`, its tile zoom, and the two tiles of
/// that zoom left and right of the vertical tile edge nearest 0.01 E.
fn edge_view(scale: f64) -> (View, u8, TileId, TileId, f64) {
    let probe = View::new(LatLon::new(0.001, 0.01).expect("ll"), scale);
    let vp = render::viewport(&probe, 80, 20);
    let z = ZoomRange::new(0, 24)
        .map(|r| vp.tile_zoom(r))
        .expect("range");
    let right = TileId::containing(LatLon::wrapped(0.001, 0.01), z);
    let left = TileId::new(z, right.x() - 1, right.y()).expect("left");
    // Longitude of the shared edge.
    let n = f64::from(1u32 << z);
    let edge_lon = f64::from(right.x()) / n * 360.0 - 180.0;
    (
        View::new(LatLon::new(0.001, edge_lon).expect("ll"), scale),
        z,
        left,
        right,
        edge_lon,
    )
}

/// Pieces of one horizontal line at latitude 0.0005 crossing the edge,
/// each clipped to its tile plus a buffer of 64 units, as tippecanoe does.
fn pieces(
    layer: &str,
    attrs: &[(&str, Attr<'_>)],
    left: TileId,
    right: TileId,
    edge_lon: f64,
) -> Vec<(ViewTile, Arc<mvt::Tile>)> {
    let lat = 0.0005;
    let mut out = Vec::new();
    for id in [left, right] {
        let (x0, y) = tile_xy(id, lat, edge_lon - 0.02);
        let (x1, _) = tile_xy(id, lat, edge_lon + 0.02);
        let (a, b) = (x0.max(-64), x1.min(4096 + 64));
        let mut t = TileMaker::new(id);
        t.line_xy(layer, attrs, &[(a, y), (b, y)]);
        out.push((ViewTile { id, wrap: 0 }, decode(&t)));
    }
    out
}

fn row_with_most(c: &Canvas, colour: Colour) -> usize {
    (0..c.height())
        .max_by_key(|&y| {
            (0..c.width())
                .filter(|&x| c.pixel(x, y) == Some(colour))
                .count()
        })
        .unwrap_or(0)
}

fn row_text(c: &Canvas, y: usize, colour: Colour) -> String {
    (0..c.width())
        .map(|x| {
            if c.pixel(x, y) == Some(colour) {
                '#'
            } else {
                '.'
            }
        })
        .collect()
}

#[test]
fn a_road_crossing_a_tile_edge_is_continuous() {
    let (view, _, left, right, edge_lon) = edge_view(10.0);
    let tiles = pieces(
        "roads",
        &[("class", Attr::S("primary"))],
        left,
        right,
        edge_lon,
    );
    let src = [Source {
        role: Role::Overview,
        area: None,
        tiles,
    }];
    let c = render::render(&view, 80, 20, &src);
    let y = row_with_most(&c, Colour::Yellow);
    let row = row_text(&c, y, Colour::Yellow);
    let (cc, _) = render::crosshair(80, 20);
    // Unbroken through the edge (at the crosshair's column), from one end
    // of the line to the other.
    let first = row.find('#').expect("drawn");
    let last = row.rfind('#').expect("drawn");
    assert!(
        first < usize::from(cc) - 10 && last > usize::from(cc) + 10,
        "{row}"
    );
    assert!(!row[first..=last].contains('.'), "{row}");
}

#[test]
fn a_dashed_boundary_keeps_its_phase_across_a_tile_edge() {
    let (view, _, left, right, edge_lon) = edge_view(10.0);
    let tiles = pieces(
        "boundaries",
        &[("admin_level", Attr::I(2))],
        left,
        right,
        edge_lon,
    );
    let src = [Source {
        role: Role::Overview,
        area: None,
        tiles: tiles.clone(),
    }];
    let c = render::render(&view, 80, 20, &src);
    let y = row_with_most(&c, Colour::Magenta);
    let row = row_text(&c, y, Colour::Magenta);
    let periodic = |row: &str| {
        let b = row.as_bytes();
        let first = row.find('#').unwrap_or(0);
        let last = row.rfind('#').unwrap_or(0);
        (first..last.saturating_sub(5)).all(|x| b[x] == b[x + 5])
    };
    assert!(periodic(&row), "dashes jump at the edge:\n{row}");
    assert!(row.contains("###..###.."), "{row}");
    // The test can see the artefact: starting each piece's pattern at 0
    // (the canvas default) breaks the period at the edge.
    let vp = render::viewport(&view, 80, 20);
    let mut naive = Canvas::new(80, 20);
    for (vt, tile) in &tiles {
        let layer = &tile.layers()[0];
        let tr = vp.tile_transform(*vt, layer.extent());
        for f in layer.features() {
            for part in f.parts() {
                let pts: Vec<(f64, f64)> = part.points.iter().map(|p| tr.apply(p.x, p.y)).collect();
                naive.polyline(&pts, Colour::Magenta, LinePattern::Dashed);
            }
        }
    }
    let naive_row = row_text(
        &naive,
        row_with_most(&naive, Colour::Magenta),
        Colour::Magenta,
    );
    assert!(
        !periodic(&naive_row),
        "the naive drawing should break:\n{naive_row}"
    );
}

#[test]
fn overzoom_draws_the_same_geometry_larger() {
    // A pack with only zoom 10 tiles; one line 1024 units long.
    let id = TileId::containing(LatLon::wrapped(TESTVILLE.0, TESTVILLE.1), 10);
    let mut t = TileMaker::new(id);
    t.line_xy(
        "roads",
        &[("class", Attr::S("primary"))],
        &[(1920, 2048), (2176, 2048)],
    );
    let tile = decode(&t);
    let zoom = ZoomRange::new(0, 10).expect("range");
    let centre = {
        let n = f64::from(1u32 << 10);
        let w = resilience_core::maps::geo::World {
            x: (f64::from(id.x()) + 0.5) / n,
            y: (f64::from(id.y()) + 0.5) / n,
        };
        resilience_core::maps::geo::unproject(w)
    };
    let mut lengths = Vec::new();
    for scale in [50.0, 20.0, 10.0] {
        let view = View::new(centre, scale);
        let vp = render::viewport(&view, 170, 40);
        let Wanted::Tiles(ts) = render::wanted(&vp, zoom, None) else {
            panic!("wanted")
        };
        assert!(ts.iter().all(|vt| vt.id.z() == 10), "overzoom uses zoom 10");
        let tiles: Vec<_> = ts
            .into_iter()
            .filter(|vt| vt.id == id)
            .map(|vt| (vt, Arc::clone(&tile)))
            .collect();
        let c = render::render(
            &view,
            170,
            40,
            &[Source {
                role: Role::Overview,
                area: None,
                tiles,
            }],
        );
        let y = row_with_most(&c, Colour::Yellow);
        lengths.push(row_text(&c, y, Colour::Yellow).matches('#').count() as f64);
    }
    // 50 -> 20 m multiplies the length by 2.5, 20 -> 10 m by 2 (within a
    // pixel of rounding).
    assert!((lengths[1] / lengths[0] - 2.5).abs() < 0.1, "{lengths:?}");
    assert!((lengths[2] / lengths[1] - 2.0).abs() < 0.05, "{lengths:?}");
}

#[test]
fn underzoom_draws_nothing_and_asks_for_no_tiles() {
    let zoom = ZoomRange::new(10, 14).expect("range");
    for scale in [200_000.0, 20_000.0, 1_000.0] {
        let view = View::new(LatLon::new(49.6, 6.1).expect("ll"), scale);
        let vp = render::viewport(&view, 170, 48);
        assert_eq!(
            render::wanted(&vp, zoom, None),
            Wanted::Underzoom,
            "{scale}"
        );
    }
    // In the viewer: a pack with minzoom 10 at 20 km per pixel asks for
    // nothing at all.
    let mut t = testland("Deep", "2026-01-01");
    t.minzoom = 10;
    t.tiles.retain(|(id, _)| id.z() >= 10);
    let mut h = Harness::new(
        "underzoom",
        &[("t.mbtiles", &t)],
        Some(TESTVILLE),
        Some(20_000.0),
    );
    let _ = render(&mut h.app, 170, 48);
    assert!(!h.app.has_requests());
    assert!(h.app.visible_packs().is_empty());
    // A view far from a pack's area asks for nothing either.
    let area = Area::from_bounds(&crate::metadata::Bounds {
        west: 6.0,
        south: 49.5,
        east: 6.3,
        north: 49.7,
    });
    let far = View::new(LatLon::new(-30.0, 100.0).expect("ll"), 50.0);
    let vp = render::viewport(&far, 170, 48);
    assert_eq!(
        render::wanted(&vp, ZoomRange::new(0, 14).expect("r"), Some(&area)),
        Wanted::Outside
    );
}

#[test]
fn names_from_the_file_are_folded_and_cannot_reach_the_terminal() {
    let id = TileId::containing(LatLon::wrapped(TESTVILLE.0, TESTVILLE.1), 12);
    let mut t = TileMaker::new(id);
    t.point(
        "places",
        &[
            ("class", Attr::S("town")),
            (
                "name",
                Attr::S("\u{c9}vry\u{141}\x1b]0;pwned\x07\x1b[2J\u{202e}x \u{43c}\u{43e}\u{441}"),
            ),
        ],
        (49.6125, 6.1319),
    );
    let tile = decode(&t);
    let view = View::new(LatLon::new(TESTVILLE.0, TESTVILLE.1).expect("ll"), 20.0);
    let vp = render::viewport(&view, 80, 20);
    let Wanted::Tiles(ts) = render::wanted(&vp, ZoomRange::new(0, 12).expect("r"), None) else {
        panic!("wanted")
    };
    let tiles = ts
        .into_iter()
        .filter(|vt| vt.id == id)
        .map(|vt| (vt, Arc::clone(&tile)))
        .collect();
    let c = render::render(
        &view,
        80,
        20,
        &[Source {
            role: Role::Overview,
            area: None,
            tiles,
        }],
    );
    let text = c.text();
    // É is in the console font and kept; Ł is not and folds to L.
    assert!(text.contains("\u{c9}vryL]0;pwned[2Jx ?"), "{text}");
    for cell in c.cells() {
        assert!(is_console_char(cell.ch), "{:?}", cell.ch);
    }
    // And through the whole viewer, with a hostile pack title.
    let mut spec = testland("Evil\x1b[31m\u{202e}name", "2026-01-01");
    spec.attribution = Some("<a href=\"x\">\u{a9} OSM\x1b[2J</a>".into());
    let mut h = Harness::new(
        "hostile",
        &[("t.mbtiles", &spec)],
        Some(TESTVILLE),
        Some(50.0),
    );
    let s = h.screen(100, 24);
    assert!(
        s.contains("Map: Evil?[31m?name") || s.contains("Map: Evil[31mname"),
        "{s}"
    );
    assert!(s.contains("(c) OSM?[2J"), "{s}");
    h.key(KeyCode::Char('i'));
    let s = h.screen(100, 60);
    assert_console_glyphs(&s);
    assert!(!s.contains('\x1b'));
}

// ---------------------------------------------------------------------------
// Every screen, console glyphs only; dumps
// ---------------------------------------------------------------------------

fn every_screen(h: &mut Harness, w: u16, hh: u16) -> Vec<String> {
    let mut out = vec![h.screen(w, hh)];
    for keys in [
        &[KeyCode::Char('i')][..],
        &[KeyCode::Esc, KeyCode::Char('L')],
        &[KeyCode::Char('a')],
        &[KeyCode::Esc, KeyCode::Char('?')],
        &[KeyCode::Esc, KeyCode::Char('d')],
        &[KeyCode::Char('d')],
    ] {
        for k in keys {
            h.key(*k);
        }
        out.push(h.screen(w, hh));
    }
    out
}

#[test]
fn only_console_font_glyphs_are_drawn_on_every_screen() {
    let (w, t) = (world(), testland("Testland", "2026-01-01"));
    let d = damaged_testland();
    for (w_, h_) in [(80, 24), (60, 20), (160, 50)] {
        let mut scenes = vec![
            Harness::new(
                "glyph-a",
                &[("w.mbtiles", &w), ("t.mbtiles", &t)],
                Some(TESTVILLE),
                Some(5.0),
            ),
            Harness::new(
                "glyph-b",
                &[("w.mbtiles", &w), ("t.mbtiles", &t)],
                Some((49.0, 6.1)),
                Some(50.0),
            ),
            Harness::new(
                "glyph-c",
                &[("w.mbtiles", &w), ("t.mbtiles", &t)],
                Some((49.0, 6.1)),
                Some(5_000.0),
            ),
            Harness::new("glyph-d", &[("w.mbtiles", &w)], None, Some(200_000.0)),
            Harness::new("glyph-e", &[("d.mbtiles", &d)], Some(TESTVILLE), Some(20.0)),
            Harness::new("glyph-f", &[], None, None),
        ];
        for h in &mut scenes {
            for s in every_screen(h, w_, h_) {
                assert_console_glyphs(&s);
            }
        }
    }
}

/// `RESILIENCE_DUMP=1 cargo test -p resilience-maps dump_viewer_screens -- --nocapture`
#[test]
fn dump_viewer_screens() {
    if std::env::var_os("RESILIENCE_DUMP").is_none() {
        return;
    }
    let (w, t) = (world(), testland("Testland", "2026-01-01"));
    let mut h = Harness::new(
        "dump",
        &[("w.mbtiles", &w), ("t.mbtiles", &t)],
        Some(TESTVILLE),
        Some(10.0),
    );
    println!("{}\n", h.coloured(100, 30));
    h.key(KeyCode::Char('-'));
    h.key(KeyCode::Char('-'));
    h.key(KeyCode::Char('-'));
    println!("{}\n", h.coloured(100, 30));
    for _ in 0..5 {
        h.key(KeyCode::Char('-'));
    }
    println!("{}\n", h.coloured(100, 30));
    let mut out = Harness::new(
        "dump-out",
        &[("w.mbtiles", &w), ("t.mbtiles", &t)],
        Some((49.495, 6.13)),
        Some(50.0),
    );
    println!("{}\n", out.coloured(100, 30));
    h.key(KeyCode::Char('i'));
    println!("{}\n", h.screen(100, 40));
    h.key(KeyCode::Esc);
    h.key(KeyCode::Char('L'));
    println!("{}\n", h.screen(100, 40));
    h.key(KeyCode::Esc);
    h.key(KeyCode::Char('?'));
    println!("{}\n", h.screen(80, 30));
    let mut none = Harness::new("dump-none", &[], None, None);
    println!("{}\n", none.screen(80, 24));
    println!("{}\n", h.screen(39, 11));
}

#[test]
fn a_regional_pack_s_sea_is_blue_at_street_scale() {
    // Testland with sea south of 49.55 (as the pack builder cuts
    // OpenStreetMap's water polygons to a region), and without.
    let sea = |t: &mut TileMaker| {
        testland_features(t);
        t.polygon(
            "water",
            &[("class", Attr::S("sea"))],
            &[(49.50, 6.0), (49.50, 6.3), (49.55, 6.3), (49.55, 6.0)],
        );
    };
    let mut with = PackSpec::detailed("Testland", TESTLAND);
    with.center = Some((TESTVILLE.1, TESTVILLE.0, 12));
    let with = spread(with, 0..=13, TESTLAND, sea);
    let without = testland("Testland", "2026-01-01");
    let blue_cells = |spec: &PackSpec, tag: &str| {
        let mut h = Harness::new(
            tag,
            &[("w.mbtiles", &world()), ("t.mbtiles", spec)],
            Some((49.525, 6.05)),
            Some(50.0),
        );
        let s = h.coloured(80, 24);
        let map: String = s.lines().take(18).collect();
        (map.matches('b').count(), s)
    };
    let (b, s) = blue_cells(&with, "sea-with");
    assert!(b > 18 * 80 / 2, "the sea is drawn ({b} blue cells)\n{s}");
    let (b, s) = blue_cells(&without, "sea-without");
    assert!(
        b < 18 * 80 / 10,
        "without sea polygons it stays land ({b})\n{s}"
    );
}

/// Review finding (2026-11-21): a pack the viewer shows as damaged, or one
/// still being checked, must not answer searches or "nearby"; neither must
/// the reader keep a search waiting forever once it is gone.
#[test]
fn damaged_packs_are_not_searched_and_a_lost_reader_ends_a_search() {
    let t = testland("Testland", "2026-01-01").place(
        "settlement",
        "town",
        "Testville",
        None,
        TESTVILLE,
        1,
    );
    let w = world().place("settlement", "city", "Bigcity", None, (48.5, 4.0), 1);
    let mut h = Harness::new(
        "damaged-search",
        &[("w.mbtiles", &w), ("t.mbtiles", &t)],
        Some(TESTVILLE),
        Some(50.0),
    );
    // The detailed pack's check fails (as a damaged file on a stick would).
    h.app.on_response(Response::Checked {
        index: 1,
        result: Err("the integrity check failed: page 3 is damaged".into()),
    });
    h.key(KeyCode::Char('/'));
    type_text(&mut h, "testville");
    h.key(KeyCode::Enter);
    let reqs = h.app.take_requests();
    let asked: Vec<&Vec<usize>> = reqs
        .iter()
        .filter_map(|r| match r {
            Request::Search { packs, .. } => Some(packs),
            _ => None,
        })
        .collect();
    assert_eq!(asked, [&vec![0usize]], "only the overview is asked");
    for r in h.worker.handle_all(reqs) {
        h.app.on_response(r);
    }
    assert!(
        h.app.find.results.is_empty(),
        "nothing from the damaged pack"
    );
    h.key(KeyCode::Esc);
    h.key(KeyCode::Char('n'));
    let near: Vec<Vec<usize>> = h
        .app
        .take_requests()
        .into_iter()
        .filter_map(|r| match r {
            Request::Nearby { packs, .. } => Some(packs),
            _ => None,
        })
        .collect();
    assert!(near.iter().all(|p| !p.contains(&1)), "{near:?}");

    // A search waiting when the reader thread dies ends with every map
    // named as unreadable, not "Searching..." forever.
    h.key(KeyCode::Esc);
    h.key(KeyCode::Char('/'));
    h.key(KeyCode::Enter);
    let _ = h.app.take_requests();
    assert!(matches!(
        h.app.find.status,
        crate::app::FindStatus::Searching { .. }
    ));
    h.app.on_reader_lost();
    let s = flat(&h.screen(100, 30));
    assert!(!s.contains("Searching..."), "{s}");
    assert!(s.contains("Could not be searched: World overview"), "{s}");
}

// The graphical viewer's scene (ADR 0026 amendment 1): the same tiles and
// rules as the block-character map, as vector shapes.
#[test]
fn scene_has_the_lake_the_road_and_names() {
    use crate::scene::{Shape, plan_size};
    let t = testland("Testland", "2026-01-01");
    let mut h = Harness::new("scene", &[("t.mbtiles", &t)], Some(TESTVILLE), Some(20.0));
    let _ = h.app.map_scene(1200, 800, crate::scene::LABEL_CELL);
    h.pump();
    let s = h.app.map_scene(1200, 800, crate::scene::LABEL_CELL);
    let (c, r) = plan_size(1200, 800);
    assert_eq!(
        (s.width, s.height),
        (2 * u32::from(c / 2) + 1, 2 * (2 * u32::from(r / 2) + 1))
    );
    assert!(
        s.shapes
            .iter()
            .any(|x| matches!(x, Shape::Line { road: true, .. })),
        "{:?}",
        s.shapes.len()
    );
    // Every point is finite: the front end draws them as given.
    for x in &s.shapes {
        let pts: Vec<_> = match x {
            Shape::Fill { rings, .. } | Shape::Cover { rings } => rings.concat(),
            Shape::Line { points, .. } | Shape::Edge { points } => points.clone(),
        };
        assert!(pts.iter().all(|p| p.0.is_finite() && p.1.is_finite()));
    }
    let texts: Vec<&str> = s
        .labels
        .iter()
        .filter_map(|l| l.text.as_ref().map(|(_, t)| t.as_str()))
        .collect();
    // A vertical road gets no name (only nearly horizontal stretches do).
    assert!(texts.iter().any(|t| t.contains("Testville")), "{texts:?}");
    assert!(!s.truncated);
    // Nothing more to read: the scene used the planned tiles.
    assert!(!h.app.has_requests());
}

#[test]
fn drag_and_wheel_zoom_move_the_view() {
    let t = testland("Testland", "2026-01-01");
    let mut h = Harness::new("drag", &[("t.mbtiles", &t)], Some(TESTVILLE), Some(20.0));
    let c0 = h.app.view().center();
    // Dragging right moves the map right: the centre goes west.
    h.app.drag(100.0, 0.0);
    assert!(h.app.view().center().lon() < c0.lon());
    h.app.drag(-100.0, 0.0);
    assert!((h.app.view().center().lon() - c0.lon()).abs() < 1e-9);
    // Zooming around the centre keeps it; the scale changes smoothly.
    h.app.zoom_by(1.25, 0.0, 0.0);
    assert!((h.app.view().scale() - 25.0).abs() < 1e-6);
    assert!((h.app.view().center().lat() - c0.lat()).abs() < 1e-9);
    // Zooming around another point moves the centre towards it.
    h.app.zoom_by(0.5, 200.0, 0.0);
    assert!(h.app.view().center().lon() > c0.lon());
    // Garbage is ignored; other screens ignore the mouse.
    let before = *h.app.view();
    h.app.zoom_by(f64::NAN, 0.0, 0.0);
    h.app.drag(f64::INFINITY, 0.0);
    h.app.screen = Screen::Help { scroll: 0 };
    h.app.drag(10.0, 10.0);
    assert_eq!(*h.app.view(), before);
}
