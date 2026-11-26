//! `resilience-maps-gui` — the graphical map viewer of Resilience OS
//! (ADR 0016, ADR 0026 amendment 1).
//!
//! ```text
//! resilience-maps-gui [--pack FILE]... [--at LAT,LON] [--scale M] [--search WORDS]
//! ```
//!
//! The same arguments, packs, keys and screens as `resilience-maps view`,
//! drawn with iced in the graphical session: smooth vector shapes, the
//! mouse drags the map and the wheel zooms around the pointer.
//!
//! Map files are untrusted input. As in the terminal viewer this process
//! runs as the user, lowers its own address-space limit to 1 GiB **before
//! it reads any file** (and before iced starts), opens packs only through
//! the hardened reader (`resilience_maps::pack`) on its reader thread,
//! starts no program and writes only its last position. A hostile file can
//! stop this viewer, never the graphical interface that started it.
//!
//! Exit status: as `resilience-maps view` (0, 1 error, 2 every pack
//! refused, 64 usage).

mod draw;
mod keys;
mod panel;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iced::widget::{canvas, column, container, row, text};
use iced::{Color, Element, Font, Length, Subscription, Task, keyboard, window};
use resilience_core::config::{self, ThemeName, UserSettings};
use resilience_core::packs::PACKS_CONFIG_PATH;
use resilience_maps::app::{Effect, Level, MapApp, Screen, Start};
use resilience_maps::loader::{self, Catalog, Channel, Worker};
use resilience_maps::rlimit::{ADDRESS_SPACE_LIMIT, limit_address_space};
use resilience_maps::scene::Scene;
use resilience_maps::{args, position, ui};

const EXIT_ERROR: u8 = 1;
const EXIT_USAGE: u8 = 64;

const USAGE: &str = "\
Usage:
  resilience-maps-gui [--pack FILE]... [--at LAT,LON] [--scale M] [--search WORDS]";

/// The bundled fonts' directory (set by the graphical session).
const FONTS_VAR: &str = "RESILIENCE_GUI_FONTS";
const UI_FONT: Font = Font::with_name("Inter Variable");
const MONO_FONT: Font = Font::with_name("DejaVu Sans Mono");

/// Set by SIGTERM, SIGHUP and SIGINT; the next tick leaves cleanly.
static STOP: AtomicBool = AtomicBool::new(false);
/// The exit status decided inside the event loop.
static EXIT: AtomicU8 = AtomicU8::new(0);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

fn main() -> ExitCode {
    // First, before any file is read or any thread exists: glibc keeps
    // one malloc arena per thread, each reserving address space; two are
    // plenty and keep the 1 GiB bound for the data.
    // SAFETY: mallopt only changes the allocator's parameters; called
    // before any other thread starts.
    unsafe {
        libc::mallopt(libc::M_ARENA_MAX, 2);
    }
    if let Err(e) = limit_address_space(ADDRESS_SPACE_LIMIT) {
        eprintln!("Error: cannot limit memory use: {e}");
        return ExitCode::from(EXIT_ERROR);
    }
    let argv: Vec<OsString> = std::env::args_os().skip(1).collect();
    match argv.first().and_then(|a| a.to_str()) {
        Some("--version") => {
            println!("resilience-maps-gui {}", resilience_core::VERSION);
            return ExitCode::SUCCESS;
        }
        Some("--help" | "-h") => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        _ => {}
    }
    let a = match args::parse_view(&argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("resilience-maps-gui: {e}\n\n{USAGE}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let home = config::home_dir();
    let saved_at = position::path(&config::state_home(&home));
    let (settings, _) = UserSettings::load(&UserSettings::path(&config::config_home(&home)));
    let catalog = Catalog::load(Path::new(PACKS_CONFIG_PATH));
    let app = MapApp::new(Start {
        packs: a.packs,
        at: a.at,
        scale: a.scale,
        search: a.search,
        saved: position::load(&saved_at),
        theme: settings.theme,
        now: Instant::now(),
    });
    let channel = match loader::spawn(Worker::new(catalog)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: cannot start the map reader: {e}");
            return ExitCode::from(EXIT_ERROR);
        }
    };
    for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        // SAFETY: the handler only stores to an atomic, which is
        // async-signal-safe.
        unsafe {
            libc::signal(sig, on_signal as *const () as libc::sighandler_t);
        }
    }
    // The viewer state outlives iced's run: the position is saved after it.
    let shared = Arc::new(Mutex::new(Some((app, channel))));
    let result = run(shared.clone(), settings.theme);
    let Some((app, channel)) = shared.lock().ok().and_then(|mut g| g.take()) else {
        return ExitCode::from(EXIT_ERROR);
    };
    drop(channel);
    if let Some(v) = app.view_to_save()
        && let Err(e) = position::save(&saved_at, &v)
    {
        eprintln!("Note: the map position could not be saved: {e}");
    }
    for f in app.failures() {
        eprintln!("Refused: {f}");
    }
    match result {
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::from(EXIT_ERROR)
        }
        Ok(()) => ExitCode::from(EXIT.load(Ordering::SeqCst).max(app.exit_code())),
    }
}

type Shared = Arc<Mutex<Option<(MapApp, Channel)>>>;

fn run(shared: Shared, theme: ThemeName) -> iced::Result {
    let boot = move || Viewer::new(shared.clone(), theme);
    let mut a = iced::application(boot, Viewer::update, Viewer::view)
        .title("Map")
        .subscription(Viewer::subscription)
        .theme(Viewer::theme)
        .window(window::Settings {
            size: iced::Size::new(1280.0, 800.0),
            decorations: false,
            ..window::Settings::default()
        });
    for f in load_fonts() {
        a = a.font(f);
    }
    a.default_font(UI_FONT).run()
}

fn load_fonts() -> Vec<Vec<u8>> {
    let Some(dir) = std::env::var_os(FONTS_VAR).map(PathBuf::from) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "ttf" || x == "otf")
            && let Ok(b) = std::fs::read(&p)
        {
            out.push(b);
        }
    }
    out
}

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    Key(keyboard::Event),
    Resized(iced::Size),
    /// The map follows the pointer by this many scene pixels.
    Drag(f64, f64),
    /// Wheel lines (positive = towards the user = zoom in) and the pointer's
    /// offset from the centre in scene pixels.
    Wheel(f64, f64, f64),
}

struct Viewer {
    shared: Shared,
    theme: ThemeName,
    size: iced::Size,
    scene: Option<Scene>,
    /// The scene as drawn, cleared when it changes.
    cache: canvas::Cache,
    /// The scene's enlargement on screen (the planner's views are at most
    /// 1023 pixels wide).
    zoom: f32,
    panel: Vec<Vec<panel::Run>>,
    readout: Vec<String>,
    status: Option<(Level, String)>,
    hints: String,
    screen: Screen,
}

/// The enlargement for a window of `w` x `h` pixels: at least 1, and
/// enough for the scene to fit the planner's bound.
fn zoom_for(w: f32, h: f32) -> f32 {
    let max = (resilience_core::maps::geo::MAX_VIEW_PIXELS - 2) as f32;
    let k = (w / max).max(h / max).max(1.0);
    if k.is_finite() { k } else { 1.0 }
}

/// Height of the readout and status bar under the map.
const BAR_H: f32 = 76.0;

impl Viewer {
    fn new(shared: Shared, theme: ThemeName) -> (Viewer, Task<Message>) {
        let v = Viewer {
            shared,
            theme,
            size: iced::Size::new(1280.0, 800.0),
            scene: None,
            cache: canvas::Cache::new(),
            zoom: 1.0,
            panel: Vec::new(),
            readout: Vec::new(),
            status: None,
            hints: String::new(),
            screen: Screen::Map,
        };
        // No resize event comes at start: ask for the window's size.
        let size = window::latest()
            .and_then(window::size)
            .map(Message::Resized);
        (v, size)
    }

    fn theme(&self) -> iced::Theme {
        iced::Theme::custom(
            "Resilience",
            iced::theme::Palette {
                background: Color::from_rgb8(0x11, 0x13, 0x17),
                text: Color::from_rgb8(0xf1, 0xf3, 0xf5),
                primary: accent(self.theme),
                success: Color::from_rgb8(0x4a, 0xde, 0x80),
                warning: Color::from_rgb8(0xfb, 0xbf, 0x24),
                danger: Color::from_rgb8(0xf8, 0x71, 0x71),
            },
        )
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            iced::time::every(Duration::from_millis(50)).map(|_| Message::Tick),
            // Every key, also those a widget saw first (the canvas takes
            // none, but iced may mark presses as captured).
            iced::event::listen_with(|e, _status, _window| match e {
                iced::Event::Keyboard(k) => Some(Message::Key(k)),
                _ => None,
            }),
            window::resize_events().map(|(_, s)| Message::Resized(s)),
        ])
    }

    fn update(&mut self, m: Message) -> Task<Message> {
        if STOP.load(Ordering::SeqCst) {
            return iced::exit();
        }
        let shared = self.shared.clone();
        let Ok(mut guard) = shared.lock() else {
            EXIT.store(EXIT_ERROR, Ordering::SeqCst);
            return iced::exit();
        };
        let Some((app, ch)) = guard.as_mut() else {
            return iced::exit();
        };
        match m {
            Message::Tick => {
                app.on_tick(Instant::now());
                match ch.ready() {
                    Ok(rs) => {
                        for r in rs {
                            app.on_response(r);
                        }
                    }
                    Err(loader::ReaderGone) => app.on_reader_lost(),
                }
            }
            Message::Key(ev) => {
                if let Some(k) = keys::from_iced(&ev)
                    && app.handle_key(k) == Effect::Quit
                {
                    return iced::exit();
                }
            }
            Message::Resized(s) => {
                self.size = s;
                app.dirty = true;
            }
            Message::Drag(dx, dy) => app.drag(dx, dy),
            Message::Wheel(lines, dx, dy) => app.zoom_by(1.25f64.powf(-lines), dx, dy),
        }
        if app.dirty {
            app.dirty = false;
            self.refresh(app);
        }
        for r in app.take_requests() {
            if ch.requests.send(r).is_err() {
                app.on_reader_lost();
            }
        }
        Task::none()
    }

    /// Recompute what `view` shows (drawing plans the view: the requests
    /// it queues are sent right after).
    fn refresh(&mut self, app: &mut MapApp) {
        self.screen = app.screen;
        let map_here =
            app.screen == Screen::Map && !app.slots().is_empty() && !app.nothing_to_show();
        if map_here {
            let (w, h) = (self.size.width, (self.size.height - BAR_H).max(100.0));
            let k = zoom_for(w, h);
            self.zoom = k;
            let cell = (f64::from(draw::CELL.0 / k), f64::from(draw::CELL.1 / k));
            self.scene = Some(app.map_scene((w / k) as u32, (h / k) as u32, cell));
            self.cache.clear();
            let cols = (w / 8.0).clamp(40.0, 400.0) as u16;
            self.readout = ui::readout(app, cols);
            self.status = app.status();
            self.hints = format!(
                "Drag to move  Wheel Zoom  {}",
                ui::hints(usize::from(cols).saturating_sub(24))
            );
            self.panel.clear();
        } else {
            self.scene = None;
            let (c, r) = panel::grid_for(self.size.width, self.size.height);
            self.panel = panel::render(app, c, r);
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let bg = Color::from_rgb8(0x11, 0x13, 0x17);
        let body: Element<'_, Message> = match &self.scene {
            Some(scene) => {
                let map = canvas(draw::MapCanvas {
                    scene,
                    zoom: self.zoom,
                    font: UI_FONT,
                    cache: &self.cache,
                })
                .width(Length::Fill)
                .height(Length::Fill);
                let dim = Color::from_rgb8(0x9a, 0xa1, 0xab);
                let mut bar = column![].spacing(2).padding([6, 12]);
                for l in &self.readout {
                    bar = bar.push(
                        text(l.clone())
                            .size(14)
                            .font(MONO_FONT)
                            .color(accent(self.theme)),
                    );
                }
                if let Some((lvl, t)) = &self.status {
                    let c = match lvl {
                        Level::Info => Color::from_rgb8(0xf1, 0xf3, 0xf5),
                        Level::Warn => Color::from_rgb8(0xfb, 0xbf, 0x24),
                        Level::Error => Color::from_rgb8(0xf8, 0x71, 0x71),
                    };
                    bar = bar.push(text(t.clone()).size(14).color(c));
                }
                bar = bar.push(row![text(self.hints.clone()).size(13).color(dim)]);
                column![
                    map,
                    container(bar)
                        .width(Length::Fill)
                        .style(|_| container::Style {
                            background: Some(Color::from_rgb8(0x1b, 0x1e, 0x24).into()),
                            ..container::Style::default()
                        })
                ]
                .into()
            }
            None => container(panel::view(&self.panel, accent(self.theme), MONO_FONT))
                .padding(8)
                .into(),
        };
        container(body)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(bg.into()),
                ..container::Style::default()
            })
            .into()
    }
}

fn accent(t: ThemeName) -> Color {
    match t {
        ThemeName::Green => Color::from_rgb8(0x4a, 0xde, 0x80),
        ThemeName::Amber => Color::from_rgb8(0xfb, 0xbf, 0x24),
        ThemeName::Cyan => Color::from_rgb8(0x38, 0xbd, 0xf8),
        ThemeName::Mono => Color::from_rgb8(0xe5, 0xe5, 0xe5),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_keeps_the_scene_within_the_planner_bound() {
        assert_eq!(zoom_for(800.0, 600.0), 1.0);
        for (w, h) in [(1920.0, 1080.0), (3840.0, 2160.0), (1023.0, 4000.0)] {
            let k = zoom_for(w, h);
            assert!(w / k <= 1022.0 && h / k <= 1022.0, "{w}x{h}: {k}");
        }
        assert_eq!(zoom_for(f32::NAN, f32::NAN), 1.0);
    }
}
