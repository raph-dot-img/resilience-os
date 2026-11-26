//! The viewer's state machine, `MapApp` (ADR 0016 §7). It never touches
//! the terminal, files or threads: keys come in through
//! [`MapApp::handle_key`] (which returns an [`Effect`]), the reader's
//! answers through [`MapApp::on_response`], time through
//! [`MapApp::on_tick`]; what it wants read is queued as [`Request`]s that
//! `main.rs` (or a test) takes with [`MapApp::take_requests`].
//!
//! **Screens** ([`Screen`]): the map, the information screen (`i`), the
//! legend (`L`), the help (`?`, F1) and the search (`/`, or `--search`):
//! the words typed are turned into an FTS5 query by
//! `resilience_core::maps::search` (never passed as typed), every open
//! pack with search tables is asked by the reader, and Enter on a result
//! centres the map on it. **Nearby** (`n`) lists the points of interest
//! and peaks around the crosshair, nearest first (Tab: one category at a
//! time, most urgent first); **go to** (`g`) reads typed coordinates with
//! `geo::parse_coordinates`.
//!
//! **Packs**: each `--pack` is a [`Slot`] (opening, open, failed). An open
//! pack has a [`Role`]:
//! - an **overview** (highest zoom at most 8, like the built-in world map)
//!   is drawn first, everywhere, and never counts as detailed;
//! - a **detailed** pack has a coverage polygon: it is drawn inside it and
//!   lifts the "no detailed map here" shading there;
//! - a pack **without a coverage polygon** (a third-party or self-made file)
//!   is drawn inside the rectangle of its `bounds`. That rectangle is only
//!   an outer limit, so it is never presented as precise: its edge is drawn
//!   dotted at every scale, the information screen says the area is not
//!   stated, and while the crosshair is inside it (and outside every real
//!   coverage polygon) the status line says the map may be incomplete
//!   there. It does lift the shading inside the rectangle, or the file
//!   could not be read at all at street scales. A file that states no
//!   bounds either is drawn everywhere over the overview but never lifts
//!   the shading.
//!
//! Detailed packs are drawn by data date, newest last (on top), so where
//! two overlap the newest data shows.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use resilience_core::config::ThemeName;
use resilience_core::maps::canvas::Canvas;
use resilience_core::maps::geo::{LatLon, MAX_SCALE, View, ZOOM_LADDER};
use resilience_core::maps::label::fold_to;
use resilience_core::maps::nearby::{self, Near};
use resilience_core::maps::search::{self, Hit};
use resilience_core::maps::style::PoiCategory;

use crate::area::Area;
use crate::cache::{BadKind, Insert, Key, Lookup, TileCache};
use crate::loader::{Integrity, Request, Response, Summary, TileResult};
use crate::metadata::Profile;
use crate::render::{self, OVERVIEW_MAX_ZOOM, Role, Source, Wanted};

/// A notice stays on the status line this long.
pub const NOTICE_TIME: Duration = Duration::from_secs(8);
/// Tiles not read after this long make the status line say the file is
/// slow.
pub const SLOW_AFTER: Duration = Duration::from_secs(1);
/// The map needs at least this many cells (as the main interface).
pub const MIN_COLS: u16 = 40;
pub const MIN_ROWS: u16 = 12;

/// What the event loop must do after a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    None,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Map,
    Info {
        scroll: u16,
    },
    /// `all`: every entry, not only what is drawn at the current scale.
    Legend {
        all: bool,
        scroll: u16,
    },
    Help {
        scroll: u16,
    },
    /// Find a place (`/`): see [`Find`].
    Search,
    /// What is near the crosshair (`n`): see [`Around`].
    Nearby,
    /// Go to typed coordinates (`g`).
    GoTo,
}

/// The nearby screen's state.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Around {
    /// Where the list was made (the crosshair then).
    pub at: Option<LatLon>,
    pub found: Vec<Near>,
    /// One category only (Tab), or all.
    pub filter: Option<PoiCategory>,
    pub sel: usize,
    pub status: FindStatus,
}

impl Around {
    /// The rows shown with the filter.
    pub fn shown(&self) -> Vec<&Near> {
        self.found
            .iter()
            .filter(|n| self.filter.is_none_or(|c| n.category == c))
            .collect()
    }
}

/// The go-to dialog's state.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GoTo {
    pub input: String,
    pub error: Option<String>,
}

/// The search screen's state, kept when it is closed so that `/` shows
/// the last results again.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Find {
    pub input: String,
    pub results: Vec<Hit>,
    pub sel: usize,
    pub status: FindStatus,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum FindStatus {
    /// Nothing searched for the text as it is.
    #[default]
    Typing,
    Searching {
        generation: u64,
    },
    /// The packs searched (titles) and those that could not be.
    Done {
        query: String,
        searched: Vec<String>,
        failed: Vec<String>,
    },
}

/// How serious a status line message is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

/// An open pack as the viewer uses it.
#[derive(Debug, Clone)]
pub struct OpenPack {
    pub summary: Summary,
    pub role: Role,
    /// Where its data is drawn and what it covers: the coverage polygon,
    /// or the bounds rectangle (approximate packs), or `None` (everywhere).
    pub area: Option<Area>,
    /// Folded for the console (`label::fold_to`).
    pub title: String,
    pub attribution: String,
}

impl OpenPack {
    fn new(summary: Summary) -> OpenPack {
        let m = &summary.metadata;
        let polygon = summary.coverage.as_ref().and_then(Area::from_coverage);
        let role = if m.zoom.max() <= OVERVIEW_MAX_ZOOM {
            Role::Overview
        } else if polygon.is_some() {
            Role::Detailed
        } else {
            Role::Approximate
        };
        let area = match role {
            Role::Overview => m.bounds_given.then(|| Area::from_bounds(&m.bounds)),
            Role::Detailed => polygon,
            Role::Approximate => m.bounds_given.then(|| Area::from_bounds(&m.bounds)),
        };
        let file = summary
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let title = fold_to(&m.name, 60)
            .or_else(|| fold_to(&file, 60))
            .unwrap_or_else(|| "(unnamed map)".into());
        let attribution = m
            .attribution
            .as_deref()
            .map(strip_tags)
            .and_then(|a| fold_to(&a, 120))
            .unwrap_or_else(|| format!("{title} (no credit given)"));
        OpenPack {
            summary,
            role,
            area,
            title,
            attribution,
        }
    }

    /// Usable for drawing: not being checked, not damaged.
    pub fn usable(&self) -> bool {
        !matches!(
            self.summary.integrity,
            Integrity::Checking | Integrity::Damaged(_)
        )
    }

    /// Whether this pack lifts the shading at `p` (it is not an overview
    /// and `p` is inside its area).
    fn details(&self, p: LatLon) -> bool {
        self.role != Role::Overview && self.area.as_ref().is_some_and(|a| a.contains(p))
    }

    /// The data date, for ordering (missing dates sort first, as oldest).
    fn date(&self) -> &str {
        self.summary.metadata.data_date.as_deref().unwrap_or("")
    }
}

/// Attribution text often comes as HTML from other tools (`<a href=...>`):
/// keep the text only.
pub fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

#[derive(Debug, Clone)]
pub enum Slot {
    Opening { path: PathBuf },
    Open(Box<OpenPack>),
    Failed { path: PathBuf, message: String },
}

/// The tiles of the current view.
struct Plan {
    view: View,
    cols: u16,
    rows: u16,
    /// Per pack index, in drawing order: its tiles.
    sources: Vec<(usize, Vec<resilience_core::maps::geo::ViewTile>)>,
    pinned: HashSet<Key>,
    waiting: HashSet<Key>,
    sent: Instant,
    too_many: bool,
}

/// What the viewer starts with.
#[derive(Debug, Clone)]
pub struct Start {
    pub packs: Vec<PathBuf>,
    pub at: Option<LatLon>,
    pub scale: Option<f64>,
    pub search: Option<String>,
    /// The last position file's view, if any.
    pub saved: Option<View>,
    pub theme: ThemeName,
    pub now: Instant,
}

pub struct MapApp {
    pub screen: Screen,
    view: View,
    /// False until the start position is decided (it may wait for the
    /// packs to open).
    center_known: bool,
    scale_known: bool,
    pub dms: bool,
    slots: Vec<Slot>,
    cache: TileCache,
    outbox: Vec<Request>,
    generation: u64,
    plan: Option<Plan>,
    replan: bool,
    notice: Option<(Level, String, Instant)>,
    /// Packs whose damaged or slow tiles were already reported.
    reported: HashSet<(usize, bool)>,
    too_much: bool,
    reader_lost: bool,
    /// The search screen (`/`).
    pub find: Find,
    /// The nearby screen (`n`).
    pub around: Around,
    /// The go-to dialog (`g`).
    pub goto: GoTo,
    /// `--search` waits for the packs to open.
    search_pending: bool,
    search_generation: u64,
    pub theme: ThemeName,
    now: Instant,
    /// Last map size in cells, for panning by a quarter screen.
    map_size: (u16, u16),
    /// Something changed since the last draw.
    pub dirty: bool,
}

/// Scale of a tile zoom at a latitude, snapped to the ladder.
fn scale_for_zoom(z: u8, lat: f64) -> f64 {
    let m = 156_543.03 * lat.to_radians().cos() / f64::from(1u32 << z.min(24));
    crate::args::snap_scale(m.clamp(ZOOM_LADDER[0], MAX_SCALE))
}

impl MapApp {
    pub fn new(s: Start) -> MapApp {
        let center = s.at.or(s.saved.map(|v| v.center()));
        let scale = s.scale.or(s.saved.map(|v| v.scale()));
        let view = View::new(
            center.unwrap_or(LatLon::wrapped(20.0, 0.0)),
            scale.unwrap_or(MAX_SCALE),
        );
        let mut outbox = Vec::new();
        let slots = s
            .packs
            .iter()
            .enumerate()
            .map(|(index, path)| {
                outbox.push(Request::Open {
                    index,
                    path: path.clone(),
                });
                Slot::Opening { path: path.clone() }
            })
            .collect();
        MapApp {
            view,
            center_known: center.is_some(),
            scale_known: scale.is_some(),
            dms: false,
            slots,
            cache: TileCache::default(),
            outbox,
            generation: 0,
            plan: None,
            replan: true,
            notice: None,
            reported: HashSet::new(),
            too_much: false,
            reader_lost: false,
            screen: if s.search.is_some() {
                Screen::Search
            } else {
                Screen::Map
            },
            find: Find {
                input: s
                    .search
                    .as_deref()
                    .map(|t| t.chars().take(search::MAX_INPUT_CHARS).collect())
                    .unwrap_or_default(),
                ..Find::default()
            },
            search_pending: s.search.is_some(),
            around: Around::default(),
            goto: GoTo::default(),
            search_generation: 0,
            theme: s.theme,
            now: s.now,
            map_size: (80, 20),
            dirty: true,
        }
    }

    pub fn view(&self) -> &View {
        &self.view
    }

    pub fn slots(&self) -> &[Slot] {
        &self.slots
    }

    /// The open packs, with their index.
    pub fn open_packs(&self) -> impl Iterator<Item = (usize, &OpenPack)> {
        self.slots.iter().enumerate().filter_map(|(i, s)| match s {
            Slot::Open(p) => Some((i, p.as_ref())),
            _ => None,
        })
    }

    pub fn cache(&self) -> &TileCache {
        &self.cache
    }

    /// The view to save on exit (`None` before it was decided).
    pub fn view_to_save(&self) -> Option<View> {
        (self.center_known && self.scale_known).then_some(self.view)
    }

    /// Exit status: 2 when packs were given and none can be shown.
    pub fn exit_code(&self) -> u8 {
        // A pack still being checked when the user leaves is not a failure.
        let any_ok = self
            .open_packs()
            .any(|(_, p)| !matches!(p.summary.integrity, Integrity::Damaged(_)));
        let resolved = !self.slots.iter().any(|s| matches!(s, Slot::Opening { .. }));
        if !self.slots.is_empty() && resolved && !any_ok {
            2
        } else {
            0
        }
    }

    /// One line per pack that cannot be shown, for the terminal after exit.
    pub fn failures(&self) -> Vec<String> {
        let mut out = Vec::new();
        for s in &self.slots {
            match s {
                Slot::Failed { path, message } => out.push(format!(
                    "{}: {message}",
                    crate::text::clean_short(&path.to_string_lossy(), 200)
                )),
                Slot::Open(p) => {
                    if let Integrity::Damaged(m) = &p.summary.integrity {
                        out.push(format!(
                            "{}: {m}",
                            crate::text::clean_short(&p.summary.path.to_string_lossy(), 200)
                        ));
                    }
                }
                Slot::Opening { .. } => {}
            }
        }
        out
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.outbox)
    }

    /// Time passes: notices expire, a read becomes "slow". Cheap: only the
    /// time-dependent parts of the status are looked at.
    pub fn on_tick(&mut self, now: Instant) {
        let prev = self.now;
        self.now = now;
        if self
            .notice
            .as_ref()
            .is_some_and(|(_, _, at)| now.duration_since(*at) >= NOTICE_TIME)
        {
            self.notice = None;
            self.dirty = true;
        }
        if let Some(p) = &self.plan
            && !p.waiting.is_empty()
            && (prev.duration_since(p.sent) >= SLOW_AFTER)
                != (now.duration_since(p.sent) >= SLOW_AFTER)
        {
            self.dirty = true;
        }
    }

    fn notify(&mut self, level: Level, text: String) {
        self.notice = Some((level, text, self.now));
        self.dirty = true;
    }

    /// The reader thread ended (it panicked): nothing more can be read.
    pub fn on_reader_lost(&mut self) {
        if !self.reader_lost {
            self.reader_lost = true;
            self.dirty = true;
        }
        // A search or nearby lookup waiting for the reader will never get
        // an answer: say that every open map could not be read, rather
        // than "Searching..." forever.
        let failed: Vec<String> = self.open_packs().map(|(_, p)| p.title.clone()).collect();
        for (status, query) in [
            (&mut self.find.status, self.find.input.clone()),
            (&mut self.around.status, String::new()),
        ] {
            if matches!(status, FindStatus::Searching { .. }) {
                *status = FindStatus::Done {
                    query,
                    searched: Vec::new(),
                    failed: failed.clone(),
                };
            }
        }
    }

    fn title_of(&self, index: usize) -> String {
        match self.slots.get(index) {
            Some(Slot::Open(p)) => p.title.clone(),
            _ => "the map".into(),
        }
    }

    pub fn on_response(&mut self, r: Response) {
        self.dirty = true;
        match r {
            Response::Opened { index, summary } => {
                if let Some(slot) = self.slots.get_mut(index) {
                    *slot = Slot::Open(Box::new(OpenPack::new(*summary)));
                    self.replan = true;
                }
                self.decide_start();
            }
            Response::Refused { index, message, .. } => {
                if let Some(slot) = self.slots.get_mut(index) {
                    let path = match slot {
                        Slot::Opening { path } | Slot::Failed { path, .. } => path.clone(),
                        Slot::Open(p) => p.summary.path.clone(),
                    };
                    let name = file_name(&path);
                    *slot = Slot::Failed { path, message };
                    self.notify(
                        Level::Error,
                        format!("{name} cannot be shown: see i (information)."),
                    );
                }
                self.decide_start();
            }
            Response::Checking { index } => {
                if let Some(Slot::Open(p)) = self.slots.get_mut(index) {
                    p.summary.integrity = Integrity::Checking;
                    self.replan = true;
                }
            }
            Response::Checked { index, result } => {
                let title = self.title_of(index);
                if let Some(Slot::Open(p)) = self.slots.get_mut(index) {
                    match result {
                        Ok(()) => {
                            p.summary.integrity = Integrity::Passed;
                            self.replan = true;
                        }
                        Err(m) => {
                            p.summary.integrity = Integrity::Damaged(m);
                            self.cache.forget_pack(index);
                            self.replan = true;
                            self.notify(
                                Level::Error,
                                format!(
                                    "{title} is damaged and is not shown: see i (information)."
                                ),
                            );
                        }
                    }
                }
            }
            Response::Tile {
                generation,
                key,
                result,
                ..
            } => self.on_tile(generation, key, result),
            Response::Found {
                generation,
                hits,
                searched,
                failed,
            } => self.on_found(generation, hits, &searched, &failed),
            Response::NearbyFound {
                generation,
                hits,
                searched,
                failed,
            } => self.on_nearby(generation, hits, &searched, &failed),
        }
        // `--search`: once every pack has opened and finished its check
        // (a pack being checked is not searched).
        let busy = self.slots.iter().any(|s| match s {
            Slot::Opening { .. } => true,
            Slot::Open(p) => p.summary.integrity == Integrity::Checking,
            Slot::Failed { .. } => false,
        });
        if self.search_pending && !busy {
            self.search_pending = false;
            self.start_search();
        }
    }

    /// Keys of the search screen: typing edits the words, Enter searches
    /// (or, on results of these very words, goes to the chosen one).
    fn search_key(&mut self, k: KeyEvent) {
        let n = self.find.results.len();
        match k.code {
            KeyCode::Esc => self.screen = Screen::Map,
            KeyCode::Enter => {
                let current = matches!(&self.find.status,
                    FindStatus::Done { query, .. } if *query == self.find.input);
                if current && n > 0 {
                    self.go_to_result(self.find.sel.min(n - 1));
                } else {
                    self.start_search();
                }
            }
            KeyCode::Up if n > 0 => self.find.sel = self.find.sel.saturating_sub(1),
            KeyCode::Down if n > 0 => self.find.sel = (self.find.sel + 1).min(n - 1),
            KeyCode::PageUp if n > 0 => self.find.sel = self.find.sel.saturating_sub(10),
            KeyCode::PageDown if n > 0 => self.find.sel = (self.find.sel + 10).min(n - 1),
            KeyCode::Home if n > 0 => self.find.sel = 0,
            KeyCode::End if n > 0 => self.find.sel = n - 1,
            KeyCode::Backspace => {
                self.find.input.pop();
                self.find.status = FindStatus::Typing;
            }
            KeyCode::Char(c) if !c.is_control() && !k.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.find.input.chars().count() < search::MAX_INPUT_CHARS {
                    self.find.input.push(c);
                }
                self.find.status = FindStatus::Typing;
            }
            _ => {}
        }
    }

    /// Ask the reader to search the open packs for the words typed.
    fn start_search(&mut self) {
        let Some(fts) = search::fts_query(&search::tokens(&self.find.input)) else {
            self.notify(
                Level::Info,
                "Type the name of a place, a street or a point of interest.".into(),
            );
            return;
        };
        let packs: Vec<usize> = self
            .open_packs()
            .filter(|(_, p)| p.usable() && p.summary.capabilities.search.is_ok())
            .map(|(i, _)| i)
            .collect();
        if packs.is_empty() {
            self.find.results.clear();
            self.find.status = FindStatus::Done {
                query: self.find.input.clone(),
                searched: Vec::new(),
                failed: Vec::new(),
            };
            return;
        }
        self.search_generation += 1;
        self.find.status = FindStatus::Searching {
            generation: self.search_generation,
        };
        self.outbox.push(Request::Search {
            generation: self.search_generation,
            fts,
            exact: search::search_fold(&self.find.input),
            centre: self.view.center(),
            packs,
        });
    }

    fn on_found(
        &mut self,
        generation: u64,
        hits: Vec<Hit>,
        searched: &[usize],
        failed: &[(usize, String)],
    ) {
        if self.find.status != (FindStatus::Searching { generation }) {
            return; // an older search, or the words changed meanwhile
        }
        self.find.results = search::rank(hits, &self.find.input, self.view.center());
        self.find.sel = 0;
        self.find.status = FindStatus::Done {
            query: self.find.input.clone(),
            searched: searched.iter().map(|i| self.title_of(*i)).collect(),
            failed: failed.iter().map(|(i, _)| self.title_of(*i)).collect(),
        };
    }

    /// `n`: ask every pack with the R*Tree what is near the crosshair.
    fn start_nearby(&mut self) {
        let at = self.view.center();
        self.screen = Screen::Nearby;
        self.around = Around {
            at: Some(at),
            ..Around::default()
        };
        let packs: Vec<usize> = self
            .open_packs()
            .filter(|(_, p)| p.usable() && p.summary.capabilities.nearby.is_ok())
            .map(|(i, _)| i)
            .collect();
        if packs.is_empty() {
            self.around.status = FindStatus::Done {
                query: String::new(),
                searched: Vec::new(),
                failed: Vec::new(),
            };
            return;
        }
        self.search_generation += 1;
        self.around.status = FindStatus::Searching {
            generation: self.search_generation,
        };
        self.outbox.push(Request::Nearby {
            generation: self.search_generation,
            at,
            packs,
        });
    }

    fn on_nearby(
        &mut self,
        generation: u64,
        hits: Vec<Hit>,
        searched: &[usize],
        failed: &[(usize, String)],
    ) {
        if self.around.status != (FindStatus::Searching { generation }) {
            return;
        }
        let at = self.around.at.unwrap_or(self.view.center());
        self.around.found = nearby::order(hits, at);
        self.around.sel = 0;
        self.around.status = FindStatus::Done {
            query: String::new(),
            searched: searched.iter().map(|i| self.title_of(*i)).collect(),
            failed: failed.iter().map(|(i, _)| self.title_of(*i)).collect(),
        };
    }

    /// Keys of the nearby screen: choose, Tab for one category at a time,
    /// Enter to centre the map on the point.
    fn nearby_key(&mut self, code: KeyCode) {
        let n = self.around.shown().len();
        match code {
            KeyCode::Esc | KeyCode::Char('q' | 'n') => self.screen = Screen::Map,
            KeyCode::Tab | KeyCode::BackTab => {
                let cats = nearby::categories(&self.around.found);
                let pos = self
                    .around
                    .filter
                    .and_then(|f| cats.iter().position(|c| *c == f));
                self.around.filter = if code == KeyCode::Tab {
                    match pos {
                        None => cats.first().copied(),
                        Some(i) => cats.get(i + 1).copied(),
                    }
                } else {
                    match pos {
                        None => cats.last().copied(),
                        Some(0) => None,
                        Some(i) => cats.get(i - 1).copied(),
                    }
                };
                self.around.sel = 0;
            }
            KeyCode::Up | KeyCode::Char('k') if n > 0 => {
                self.around.sel = self.around.sel.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') if n > 0 => {
                self.around.sel = (self.around.sel + 1).min(n - 1)
            }
            KeyCode::PageUp if n > 0 => self.around.sel = self.around.sel.saturating_sub(10),
            KeyCode::PageDown if n > 0 => self.around.sel = (self.around.sel + 10).min(n - 1),
            KeyCode::Home if n > 0 => self.around.sel = 0,
            KeyCode::End if n > 0 => self.around.sel = n - 1,
            KeyCode::Enter if n > 0 => {
                let shown = self.around.shown();
                if let Some(p) = shown.get(self.around.sel.min(n - 1)) {
                    let (at, name) = (p.hit.at, p.hit.name.clone());
                    self.view.set_center(at);
                    if self.view.scale() > 20.0 {
                        self.view.set_scale(20.0);
                    }
                    self.center_known = true;
                    self.scale_known = true;
                    self.screen = Screen::Map;
                    self.moved();
                    let name = fold_to(&name, 60).unwrap_or_else(|| "The place".into());
                    self.notify(Level::Info, format!("{name}: at the crosshair."));
                }
            }
            _ => {}
        }
    }

    /// Keys of the go-to dialog.
    fn goto_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => self.screen = Screen::Map,
            KeyCode::Enter => {
                match resilience_core::maps::geo::parse_coordinates(&self.goto.input) {
                    Ok(at) => {
                        self.view.set_center(at);
                        self.center_known = true;
                        self.screen = Screen::Map;
                        self.goto.error = None;
                        self.moved();
                        self.notify(Level::Info, "The crosshair is on the coordinates.".into());
                    }
                    Err(e) => self.goto.error = Some(e.to_string()),
                }
            }
            KeyCode::Backspace => {
                self.goto.input.pop();
                self.goto.error = None;
            }
            KeyCode::Char(c) if !c.is_control() && !k.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.goto.input.chars().count() < resilience_core::maps::geo::MAX_COORD_INPUT {
                    self.goto.input.push(c);
                }
                self.goto.error = None;
            }
            _ => {}
        }
    }

    /// Centre the map on a result, at a scale that suits its kind.
    fn go_to_result(&mut self, i: usize) {
        let Some(h) = self.find.results.get(i).cloned() else {
            return;
        };
        self.view.set_center(h.at);
        self.view.set_scale(resilience_core::maps::geo::clamp_scale(
            h.kind.scale(&h.class),
        ));
        self.center_known = true;
        self.scale_known = true;
        self.screen = Screen::Map;
        self.moved();
        let name = fold_to(&h.name, 60).unwrap_or_else(|| "The place".into());
        self.notify(Level::Info, format!("{name}: at the crosshair."));
    }

    fn on_tile(&mut self, generation: u64, key: Key, result: TileResult) {
        if let Some(plan) = self.plan.as_mut()
            && generation == self.generation
        {
            plan.waiting.remove(&key);
        }
        let empty = HashSet::new();
        let pinned = self.plan.as_ref().map_or(&empty, |p| &p.pinned);
        match result {
            TileResult::Tile(tile, bytes) => {
                if self.cache.insert(key, tile, bytes, pinned) == Insert::TooMuch {
                    self.too_much = true;
                }
            }
            TileResult::Absent => self.cache.insert_absent(key),
            TileResult::Damaged(why) => {
                self.cache.insert_bad(key, BadKind::Damaged, self.now);
                if self.reported.insert((key.pack, true)) {
                    let title = self.title_of(key.pack);
                    self.notify(
                        Level::Warn,
                        format!("Part of {title} is damaged and is not shown ({why})."),
                    );
                }
            }
            TileResult::Slow => {
                self.cache.insert_bad(key, BadKind::Slow, self.now);
                if self.reported.insert((key.pack, false)) {
                    let title = self.title_of(key.pack);
                    self.notify(
                        Level::Warn,
                        format!(
                            "{title} is too slow to read here: the USB stick is slow or the file is damaged."
                        ),
                    );
                }
            }
        }
    }

    /// Decide the start position once every pack has answered: the first
    /// detailed pack's centre, else an overview's, else the world.
    fn decide_start(&mut self) {
        if (self.center_known && self.scale_known)
            || self.slots.iter().any(|s| matches!(s, Slot::Opening { .. }))
        {
            return;
        }
        let packs: Vec<&OpenPack> = self.open_packs().map(|(_, p)| p).collect();
        let first = packs
            .iter()
            .find(|p| p.role != Role::Overview)
            .or_else(|| packs.first());
        let (center, scale) = match first {
            Some(p) => {
                let m = &p.summary.metadata;
                let c = m
                    .center
                    .map(|c| c.at)
                    .or_else(|| p.area.as_ref().map(Area::centre))
                    .unwrap_or(LatLon::wrapped(20.0, 0.0));
                let z = m.center.and_then(|c| c.zoom);
                let s = match (z, p.role) {
                    (Some(z), _) => scale_for_zoom(z, c.lat()),
                    (None, Role::Overview) => 20_000.0,
                    (None, _) => 100.0,
                };
                (c, s)
            }
            None => (LatLon::wrapped(20.0, 0.0), 20_000.0),
        };
        if !self.center_known {
            self.view.set_center(center);
            self.center_known = true;
        }
        if !self.scale_known {
            self.view.set_scale(scale);
            self.scale_known = true;
        }
        self.replan = true;
    }

    /// Open packs in drawing order: overviews, then the others by data
    /// date (oldest first, newest on top); ties keep the command-line order
    /// with the first given on top.
    pub fn drawing_order(&self) -> Vec<usize> {
        let mut v: Vec<(usize, &OpenPack)> =
            self.open_packs().filter(|(_, p)| p.usable()).collect();
        v.sort_by(|(ia, a), (ib, b)| {
            let ra = a.role != Role::Overview;
            let rb = b.role != Role::Overview;
            ra.cmp(&rb)
                .then_with(|| {
                    if ra {
                        a.date().cmp(b.date())
                    } else {
                        std::cmp::Ordering::Equal
                    }
                })
                .then_with(|| if ra { ib.cmp(ia) } else { ia.cmp(ib) })
        });
        v.into_iter().map(|(i, _)| i).collect()
    }

    fn open_pack(&self, i: usize) -> Option<&OpenPack> {
        match self.slots.get(i) {
            Some(Slot::Open(p)) => Some(p),
            _ => None,
        }
    }

    /// Make sure the plan matches the view and the map size; queue reads.
    fn ensure_plan(&mut self, cols: u16, rows: u16) {
        self.map_size = (cols, rows);
        let fresh = self
            .plan
            .as_ref()
            .is_some_and(|p| p.view == self.view && p.cols == cols && p.rows == rows);
        if fresh && !self.replan {
            return;
        }
        self.replan = false;
        self.too_much = false;
        self.generation += 1;
        let vp = render::viewport(&self.view, cols, rows);
        let mut sources = Vec::new();
        let mut too_many = false;
        for i in self.drawing_order() {
            let Some(p) = self.open_pack(i) else { continue };
            match render::wanted(&vp, p.summary.metadata.zoom, p.area.as_ref()) {
                Wanted::Tiles(t) => sources.push((i, t)),
                Wanted::TooMany => too_many = true,
                Wanted::Underzoom | Wanted::Outside => {}
            }
        }
        // Most important first (the pack drawn on top), within the bound.
        let mut keys: Vec<Key> = Vec::new();
        for (i, tiles) in sources.iter().rev() {
            for t in tiles {
                let k = Key {
                    pack: *i,
                    tile: t.id,
                };
                if !keys.contains(&k) {
                    keys.push(k);
                }
            }
        }
        if keys.len() > render::MAX_PLANNED_TILES {
            keys.truncate(render::MAX_PLANNED_TILES);
            too_many = true;
        }
        let pinned: HashSet<Key> = keys.iter().copied().collect();
        let mut missing = Vec::new();
        for k in &keys {
            if matches!(self.cache.get(k, self.now), Lookup::Miss) {
                missing.push(*k);
            }
        }
        if !missing.is_empty() {
            self.outbox.push(Request::Tiles {
                generation: self.generation,
                tiles: missing.clone(),
            });
        }
        self.plan = Some(Plan {
            view: self.view,
            cols,
            rows,
            sources,
            pinned,
            waiting: missing.into_iter().collect(),
            sent: self.now,
            too_many,
        });
    }

    /// The map of `cols` x `rows` cells: plans, queues reads and draws
    /// what is cached.
    pub fn map_canvas(&mut self, cols: u16, rows: u16) -> Canvas {
        self.ensure_plan(cols, rows);
        if self.plan.is_none() {
            return Canvas::new(cols, rows);
        }
        let view = self.view;
        self.with_sources(|sources| render::render(&view, cols, rows, sources))
    }

    /// The map of `width` x `height` pixels as vector shapes, for the
    /// graphical viewer: plans (the same tiles as a terminal of
    /// [`scene::plan_size`](crate::scene::plan_size)), queues reads and
    /// draws what is cached.
    /// `cell` is the label grid's cell in view pixels
    /// ([`scene::LABEL_CELL`](crate::scene::LABEL_CELL) at full size).
    pub fn map_scene(&mut self, width: u32, height: u32, cell: (f64, f64)) -> crate::scene::Scene {
        let (cols, rows) = crate::scene::plan_size(width, height);
        // On a wide window the coarsest scale shows the world several
        // times, small: zooming out stops where it fills the width once.
        let wpp = self.view.world_per_pixel();
        let across = wpp * f64::from(cols);
        if across.is_finite() && across > 1.0 {
            self.view.set_scale(self.view.scale() / across);
        }
        self.ensure_plan(cols, rows);
        let view = self.view;
        self.with_sources(|sources| crate::scene::scene(&view, cols, rows, sources, cell))
    }

    /// The planned sources with their cached tiles.
    fn with_sources<T>(&mut self, f: impl FnOnce(&[Source<'_>]) -> T) -> T {
        let Some(plan) = self.plan.as_ref() else {
            return f(&[]);
        };
        let mut lists = Vec::new();
        for (i, tiles) in &plan.sources {
            let mut got = Vec::new();
            for t in tiles {
                if let Lookup::Tile(tile) = self.cache.get(
                    &Key {
                        pack: *i,
                        tile: t.id,
                    },
                    self.now,
                ) {
                    got.push((*t, tile));
                }
            }
            lists.push((*i, got));
        }
        let sources: Vec<Source<'_>> = lists
            .into_iter()
            .filter_map(|(i, tiles)| {
                let p = match self.slots.get(i) {
                    Some(Slot::Open(p)) => p,
                    _ => return None,
                };
                Some(Source {
                    role: p.role,
                    area: p.area.as_ref(),
                    tiles,
                })
            })
            .collect();
        f(&sources)
    }

    /// Packs with something in view, top first.
    pub fn visible_packs(&self) -> Vec<usize> {
        self.plan
            .as_ref()
            .map(|p| p.sources.iter().rev().map(|(i, _)| *i).collect())
            .unwrap_or_default()
    }

    /// The credit of what is in view, folded for the console.
    pub fn attribution(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        for i in self.visible_packs() {
            if let Some(p) = self.open_pack(i)
                && !parts.contains(&p.attribution)
            {
                parts.push(p.attribution.clone());
            }
        }
        parts.join("; ")
    }

    /// The pack under the crosshair: the top detailed one there, else an
    /// overview in view.
    pub fn active_pack(&self) -> Option<&OpenPack> {
        let c = self.view.center();
        let visible = self.visible_packs();
        visible
            .iter()
            .filter_map(|i| self.open_pack(*i))
            .find(|p| p.details(c))
            .or_else(|| {
                visible
                    .iter()
                    .filter_map(|i| self.open_pack(*i))
                    .find(|p| p.role == Role::Overview || p.area.is_none())
            })
    }

    /// The status line: the most important message now.
    pub fn status(&self) -> Option<(Level, String)> {
        if self.reader_lost {
            return Some((
                Level::Error,
                "The map reader stopped. Press q to leave and open the map again.".into(),
            ));
        }
        if let Some((l, t, _)) = &self.notice {
            return Some((*l, t.clone()));
        }
        let open: Vec<(usize, &OpenPack)> = self.open_packs().collect();
        if let Some((_, p)) = open
            .iter()
            .find(|(_, p)| p.summary.integrity == Integrity::Checking)
        {
            return Some((
                Level::Info,
                format!(
                    "Checking the file {}... the map appears when it is done.",
                    p.title
                ),
            ));
        }
        if let Some(plan) = &self.plan {
            if !plan.waiting.is_empty() && self.now.duration_since(plan.sent) >= SLOW_AFTER {
                let title = plan
                    .waiting
                    .iter()
                    .next()
                    .map_or_else(String::new, |k| self.title_of(k.pack));
                return Some((
                    Level::Warn,
                    format!("Reading {title} is slow: a slow USB stick or a damaged file."),
                ));
            }
            if self.too_much || plan.too_many {
                return Some((
                    Level::Warn,
                    "Too much map data to show at once here: zoom in (+).".into(),
                ));
            }
        }
        let c = self.view.center();
        let usable: Vec<&OpenPack> = open
            .iter()
            .map(|(_, p)| *p)
            .filter(|p| p.usable())
            .collect();
        if !usable.is_empty()
            && render::fine(self.view.scale())
            && !usable.iter().any(|p| p.details(c))
        {
            let text = match usable.iter().find(|p| p.role == Role::Overview) {
                Some(o) => format!("No detailed map here: world overview only ({}).", o.title),
                None => "No detailed map here: no open map covers this place.".into(),
            };
            return Some((Level::Warn, text));
        }
        let precise = usable
            .iter()
            .any(|p| p.role == Role::Detailed && p.details(c));
        if !precise
            && let Some(p) = usable
                .iter()
                .find(|p| p.role == Role::Approximate && (p.area.is_none() || p.details(c)))
        {
            return Some((
                Level::Warn,
                format!(
                    "{} does not say which area it covers: the map may be incomplete here.",
                    p.title
                ),
            ));
        }
        if let Some(p) = usable
            .iter()
            .find(|p| p.summary.integrity == Integrity::NotChecked)
        {
            return Some((
                Level::Info,
                format!(
                    "{} was not checked. Press c to check the file (this takes a while).",
                    p.title
                ),
            ));
        }
        if let Some(plan) = &self.plan
            && !plan.waiting.is_empty()
        {
            return Some((Level::Info, "Reading the map...".into()));
        }
        None
    }

    /// Move the map by `(dx, dy)` view pixels (a mouse drag: the map
    /// follows the pointer). Map screen only; non-finite values are ignored.
    pub fn drag(&mut self, dx: f64, dy: f64) {
        if self.screen != Screen::Map || !(dx.is_finite() && dy.is_finite()) {
            return;
        }
        self.view.pan_pixels(-dx, -dy);
        self.moved();
    }

    /// Zoom by `factor` (> 1 zooms out) keeping the place at `(dx, dy)` view
    /// pixels from the centre under the pointer (the mouse wheel). The
    /// scale stays within the viewer's bounds.
    pub fn zoom_by(&mut self, factor: f64, dx: f64, dy: f64) {
        if self.screen != Screen::Map
            || !(factor.is_finite() && factor > 0.0 && dx.is_finite() && dy.is_finite())
        {
            return;
        }
        self.view.pan_pixels(dx, dy);
        self.view.set_scale(self.view.scale() * factor);
        self.view.pan_pixels(-dx, -dy);
        self.moved();
    }

    fn pan_quarter(&mut self, dx: f64, dy: f64) {
        let (cols, rows) = self.map_size;
        self.view.pan_pixels(
            dx * f64::from(cols.max(4)) / 4.0,
            dy * f64::from(rows.max(4)) * 2.0 / 4.0,
        );
    }

    fn moved(&mut self) {
        self.center_known = true;
        self.scale_known = true;
        self.dirty = true;
    }

    pub fn handle_key(&mut self, k: KeyEvent) -> Effect {
        if k.kind == KeyEventKind::Release {
            return Effect::None;
        }
        self.dirty = true;
        if k.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(k.code, KeyCode::Char('c' | 'q' | 'd'))
        {
            return Effect::Quit;
        }
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        match self.screen {
            Screen::Map => self.map_key(k.code, shift),
            Screen::Search => {
                self.search_key(k);
                Effect::None
            }
            Screen::Nearby => {
                self.nearby_key(k.code);
                Effect::None
            }
            Screen::GoTo => {
                self.goto_key(k);
                Effect::None
            }
            Screen::Info { scroll } | Screen::Help { scroll } | Screen::Legend { scroll, .. } => {
                let scroll = match k.code {
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Backspace => {
                        self.screen = Screen::Map;
                        return Effect::None;
                    }
                    KeyCode::Char('i') if matches!(self.screen, Screen::Info { .. }) => {
                        self.screen = Screen::Map;
                        return Effect::None;
                    }
                    KeyCode::Char('L') if matches!(self.screen, Screen::Legend { .. }) => {
                        self.screen = Screen::Map;
                        return Effect::None;
                    }
                    KeyCode::Char('a') => {
                        if let Screen::Legend { all, .. } = self.screen {
                            self.screen = Screen::Legend {
                                all: !all,
                                scroll: 0,
                            };
                        }
                        return Effect::None;
                    }
                    KeyCode::Up | KeyCode::Char('k') => scroll.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => scroll.saturating_add(1),
                    KeyCode::PageUp => scroll.saturating_sub(10),
                    KeyCode::PageDown | KeyCode::Char(' ') => scroll.saturating_add(10),
                    KeyCode::Home => 0,
                    KeyCode::End => u16::MAX,
                    _ => scroll,
                };
                self.screen = match self.screen {
                    Screen::Info { .. } => Screen::Info { scroll },
                    Screen::Help { .. } => Screen::Help { scroll },
                    Screen::Legend { all, .. } => Screen::Legend { all, scroll },
                    Screen::Map => Screen::Map,
                    s @ (Screen::Search | Screen::Nearby | Screen::GoTo) => s,
                };
                Effect::None
            }
        }
    }

    /// Clamp a screen's scroll to its content (the renderer calls this).
    pub fn clamp_scroll(&mut self, max: u16) {
        match &mut self.screen {
            Screen::Info { scroll } | Screen::Help { scroll } | Screen::Legend { scroll, .. } => {
                *scroll = (*scroll).min(max);
            }
            Screen::Map | Screen::Search | Screen::Nearby | Screen::GoTo => {}
        }
    }

    fn map_key(&mut self, code: KeyCode, shift: bool) -> Effect {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => return Effect::Quit,
            KeyCode::Left | KeyCode::Char('h') => {
                if shift {
                    self.view.pan_pixels(-1.0, 0.0);
                } else {
                    self.pan_quarter(-1.0, 0.0);
                }
                self.moved();
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if shift {
                    self.view.pan_pixels(1.0, 0.0);
                } else {
                    self.pan_quarter(1.0, 0.0);
                }
                self.moved();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if shift {
                    self.view.pan_pixels(0.0, -2.0);
                } else {
                    self.pan_quarter(0.0, -1.0);
                }
                self.moved();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if shift {
                    self.view.pan_pixels(0.0, 2.0);
                } else {
                    self.pan_quarter(0.0, 1.0);
                }
                self.moved();
            }
            KeyCode::Char('+' | '=') => {
                self.view.zoom_in();
                self.moved();
            }
            KeyCode::Char('-' | '_') => {
                self.view.zoom_out();
                self.moved();
            }
            KeyCode::Char('d') => self.dms = !self.dms,
            KeyCode::Char('i') => self.screen = Screen::Info { scroll: 0 },
            KeyCode::Char('L') => {
                self.screen = Screen::Legend {
                    all: false,
                    scroll: 0,
                }
            }
            KeyCode::Char('?') | KeyCode::F(1) => self.screen = Screen::Help { scroll: 0 },
            KeyCode::Char('c') => self.offer_check(),
            KeyCode::Char('/') => self.screen = Screen::Search,
            KeyCode::Char('n') => self.start_nearby(),
            KeyCode::Char('g') => {
                self.goto.error = None;
                self.screen = Screen::GoTo;
            }
            _ => {}
        }
        Effect::None
    }

    /// `c`: check the packs that were not checked (large files).
    fn offer_check(&mut self) {
        let todo: Vec<usize> = self
            .open_packs()
            .filter(|(_, p)| p.summary.integrity == Integrity::NotChecked)
            .map(|(i, _)| i)
            .collect();
        if todo.is_empty() {
            self.notify(Level::Info, "Every open map was already checked.".into());
            return;
        }
        for index in todo {
            if let Some(Slot::Open(p)) = self.slots.get_mut(index) {
                p.summary.integrity = Integrity::Checking;
            }
            self.outbox.push(Request::Check { index });
        }
        self.replan = true;
    }

    /// Whether packs were given but none could be opened.
    pub fn nothing_to_show(&self) -> bool {
        self.slots.is_empty()
            || (!self.slots.iter().any(|s| matches!(s, Slot::Opening { .. }))
                && !self
                    .open_packs()
                    .any(|(_, p)| p.usable() || p.summary.integrity == Integrity::Checking))
    }

    /// Whether the profile of a pack is ours.
    pub fn is_ours(p: &OpenPack) -> bool {
        p.summary.metadata.profile == Profile::Resilience1
    }
}

fn file_name(p: &std::path::Path) -> String {
    let n = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    fold_to(&n, 60).unwrap_or_else(|| "The file".into())
}

#[cfg(test)]
impl MapApp {
    /// Whether reads are queued (tests).
    pub(crate) fn has_requests(&self) -> bool {
        !self.outbox.is_empty()
    }
}
