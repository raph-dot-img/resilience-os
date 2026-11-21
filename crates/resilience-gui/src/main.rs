//! `resilience-gui` — the graphical interface of Resilience OS (ADR 0026).
//!
//! ```text
//! resilience-gui              the interface (inside cage, full screen)
//! resilience-gui run FILE     run one program request in this terminal
//!                             (started by the interface inside foot)
//! resilience-gui xkb         print "LAYOUT [VARIANT]" (XKB) for this
//!                             account's keyboard layout (session script)
//! resilience-gui --version
//! ```
//!
//! The state machine is `resilience_shell::app::App`, the same one the
//! terminal interface drives. Home, categories and search are drawn
//! natively; every other screen is the terminal renderer's output shown as
//! a text panel until it gets its own graphical view.

mod logo;
mod run;
mod style;
mod textpanel;

use std::process::ExitCode;
use std::time::Duration;

use iced::widget::{Space, button, column, container, row, scrollable, text, text_input};
use iced::{Element, Fill, Length, Subscription, Task, keyboard, window};
use resilience_core::catalog::Category;
use resilience_core::config::ThemeName;
use resilience_core::messages;
use resilience_shell::app::{App, Effect, Entry, SearchHit, Startup, View, category_description};
use resilience_shell::input::{KeyCode, KeyEvent, KeyModifiers};

use crate::style::Palette;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => match gui() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("resilience-gui: {e}");
                ExitCode::FAILURE
            }
        },
        Some("run") if args.len() == 2 => run::run_file(&args[1]),
        Some("xkb") => {
            println!("{}", xkb_layout());
            ExitCode::SUCCESS
        }
        Some("-V" | "--version") => {
            println!("resilience-gui {}", resilience_core::VERSION);
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("Usage: resilience-gui [--version | run FILE]");
            ExitCode::from(2)
        }
    }
}

/// The account's layout (Settings, else the system's) as XKB names; US
/// when it has no XKB equivalent we know.
fn xkb_layout() -> String {
    let s = Startup::load();
    let id = s.sys.keyboard.effective(s.settings.keyboard.as_deref());
    match resilience_core::keyboard::xkb_for(id) {
        Some((l, "")) => l.to_string(),
        Some((l, v)) => format!("{l} {v}"),
        None => "us".into(),
    }
}

fn gui() -> iced::Result {
    let mut a = iced::application(Gui::new, Gui::update, Gui::view)
        .title("Resilience")
        .subscription(Gui::subscription)
        .theme(Gui::theme)
        .window(window::Settings {
            size: iced::Size::new(1280.0, 800.0),
            decorations: false,
            ..window::Settings::default()
        });
    for f in style::load_fonts() {
        a = a.font(f);
    }
    a.default_font(style::UI_FONT).run()
}

#[derive(Debug, Clone)]
enum Message {
    Tick,
    Key(keyboard::Event),
    Choose(usize),
    Back,
    Home,
    Search(String),
    Resized(iced::Size),
    /// Mouse wheel over a text page: lines (positive = towards the end).
    Wheel(i32),
    RunDone(String, Result<i32, String>),
}

struct Gui {
    app: App,
    size: iced::Size,
    /// Label of the program running in its terminal window.
    running: Option<String>,
    /// The terminal renderer's output for screens without a native view.
    screen: textpanel::Screen,
}

impl Gui {
    fn new() -> (Gui, Task<Message>) {
        let mut app = App::new(Startup::load());
        app.apply_saved_layout();
        let gui = Gui {
            app,
            size: iced::Size::new(1280.0, 800.0),
            running: None,
            screen: textpanel::Screen::default(),
        };
        (gui, Task::none())
    }

    fn theme(&self) -> iced::Theme {
        style::theme(self.app.theme)
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            iced::time::every(Duration::from_millis(250)).map(|_| Message::Tick),
            keyboard::listen().map(Message::Key),
            window::resize_events().map(|(_, s)| Message::Resized(s)),
        ])
    }

    fn update(&mut self, m: Message) -> Task<Message> {
        let dbg = std::env::var_os("RESILIENCE_DEBUG").is_some() && !matches!(m, Message::Tick);
        if dbg {
            eprintln!("msg {m:?} before: depth {}", self.app.stack.len());
        }
        let task = self.step(m);
        if dbg {
            eprintln!("  after: depth {}", self.app.stack.len());
        }
        self.clamp_scroll();
        if self.wants_text_panel() {
            // Bars: ~72 px on top, ~40 px below, 28 px side padding.
            let (cols, rows) =
                textpanel::grid_for(self.size.width - 56.0, self.size.height - 120.0);
            self.screen = textpanel::render(&mut self.app, cols, rows);
        }
        task
    }

    /// Arrow keys on a grid of tiles: Left/Right move by one, Up/Down by a
    /// row. Returns false when the key is not for the grid.
    fn grid_move(&mut self, k: KeyEvent) -> bool {
        let n = match self.app.view() {
            View::Home { .. } => self.app.home_categories().len(),
            View::Category { cat, .. } => self.app.entries(*cat).len(),
            View::Library { items, .. } => items.len(),
            _ => return false,
        };
        if self.app.modal.is_some() || n == 0 || !k.modifiers.is_empty() {
            return false;
        }
        let cols = self.columns() as isize;
        let step = match k.code {
            KeyCode::Left => -1,
            KeyCode::Right => 1,
            KeyCode::Up => -cols,
            KeyCode::Down => cols,
            _ => return false,
        };
        if let Some(l) = self.app.list_mut() {
            let cur = l.selected().unwrap_or(0) as isize;
            let next = cur + step;
            if (0..n as isize).contains(&next) {
                l.select(Some(next as usize));
            }
        }
        true
    }

    fn wants_text_panel(&self) -> bool {
        self.running.is_none()
            && (self.app.modal.is_some()
                || !matches!(
                    self.app.view(),
                    View::Home { .. }
                        | View::Category { .. }
                        | View::Search { .. }
                        | View::Settings { .. }
                        | View::Handbook { .. }
                        | View::Pager { .. }
                        | View::Help { .. }
                        | View::Files { .. }
                        | View::Library { .. }
                        | View::Chats { .. }
                        | View::Nearby { .. }
                        | View::Chat { .. }
                ))
    }

    fn step(&mut self, m: Message) -> Task<Message> {
        let effect = match m {
            Message::Tick => {
                self.app.on_tick();
                Effect::None
            }
            Message::Resized(s) => {
                self.size = s;
                Effect::None
            }
            Message::Wheel(n) => {
                if let Some(sc) = self.app.scroll_mut() {
                    *sc = if n < 0 {
                        sc.saturating_sub(n.unsigned_abs() as usize)
                    } else {
                        sc.saturating_add(n as usize)
                    };
                }
                Effect::None
            }
            Message::Key(ev) => {
                if self.running.is_some() {
                    return Task::none();
                }
                let k = style::key_from_iced(&ev);
                if std::env::var_os("RESILIENCE_DEBUG").is_some() {
                    eprintln!("key {ev:?} -> {k:?}");
                }
                match k {
                    Some(k) if self.grid_move(k) => Effect::None,
                    Some(k) => self.app.handle_key(k),
                    None => Effect::None,
                }
            }
            Message::Choose(i) => {
                if self.running.is_some() {
                    return Task::none();
                }
                // In a conversation Enter writes: a click only selects the
                // message (for o open, s save, c cancel).
                if self.app.modal.is_none() && matches!(self.app.view(), View::Chat { .. }) {
                    if let Some(l) = self.app.list_mut() {
                        l.select(Some(i));
                    }
                    Effect::None
                } else {
                    self.app.choose_row(i)
                }
            }
            Message::Back => self
                .app
                .handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Message::Home => {
                self.app.go_home();
                Effect::None
            }
            Message::Search(q) => {
                if q.is_empty() && matches!(self.app.view(), View::Search { .. }) {
                    self.app
                        .handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
                } else {
                    self.app.set_search(&q);
                    Effect::None
                }
            }
            Message::RunDone(label, result) => {
                self.running = None;
                self.app.after_run(&label, &run::to_status(result));
                Effect::None
            }
        };
        match effect {
            Effect::None => Task::none(),
            Effect::Quit => iced::exit(),
            Effect::Run(req) => {
                let label = req.label.clone();
                self.running = Some(label.clone());
                Task::perform(run::in_terminal(req), move |r| {
                    Message::RunDone(label.clone(), r)
                })
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let p = Palette::of(self.app.theme);
        let body: Element<'_, Message> = if let Some(label) = &self.running {
            self.running_view(label)
        } else if self.wants_text_panel() {
            self.text_view(&p)
        } else {
            match self.app.view() {
                View::Home { .. } => self.home_view(&p),
                View::Category { cat, .. } => self.category_view(*cat, &p),
                View::Search { query, hits, .. } => self.search_view(query, hits, &p),
                View::Settings { .. } => self.settings_view(&p),
                View::Handbook { pages, error, .. } => self.handbook_view(pages, error.as_deref()),
                View::Pager { text, scroll, .. } => self.pager_view(text.clone(), *scroll, &p),
                View::Files {
                    idx, query, pick, ..
                } => self.files_view(idx, query, pick.is_some(), &p),
                View::Library { kind, items, .. } => self.library_view(*kind, items, &p),
                View::Chats { rows, status, .. } => self.chats_view(rows, status, &p),
                View::Nearby { rows, .. } => self.nearby_view(rows, &p),
                View::Chat { records, .. } => self.chat_view(records, &p),
                View::Help { topic, scroll } => {
                    self.pager_view(help_text(*topic, self.app.mode()), *scroll, &p)
                }
                _ => self.text_view(&p),
            }
        };
        let page = column![self.top_bar(&p), body, self.status_bar(&p)];
        container(page)
            .width(Fill)
            .height(Fill)
            .style(move |_| style::page(&p))
            .into()
    }

    fn top_bar(&self, p: &Palette) -> Element<'_, Message> {
        let at_home = self.app.stack.len() <= 1;
        let mut r = row![].spacing(12).align_y(iced::Center);
        if !at_home {
            r = r.push(
                button(text("\u{2190}  Back").size(16))
                    .on_press(Message::Back)
                    .padding([8, 16])
                    .style(style::ghost_button),
            );
            r = r.push(
                button(text("Home").size(16))
                    .on_press(Message::Home)
                    .padding([8, 16])
                    .style(style::ghost_button),
            );
        }
        if at_home {
            r = r.push(logo::icon(p.accent, 58.0));
        }
        r = r.push(text(self.title()).size(20).font(style::UI_FONT_BOLD));
        r = r.push(Space::new().width(Fill));
        if matches!(self.app.view(), View::Files { .. }) {
            return container(r).padding([16, 28]).height(BAR_H).into();
        }
        let query = match self.app.view() {
            View::Search { query, .. } => query.clone(),
            _ => String::new(),
        };
        r = r.push(
            text_input("Search apps, guides, places\u{2026}", &query)
                .on_input(Message::Search)
                .padding([10, 16])
                .size(16)
                .width(Length::Fixed(380.0))
                .style(style::search_box),
        );
        container(r).padding([16, 28]).height(BAR_H).into()
    }

    fn title(&self) -> String {
        match self.app.view() {
            View::Home { .. } => format!("Hello, {}", self.app.who.user),
            View::Category { cat, .. } => section_name(*cat),
            View::Search { .. } => "Search".into(),
            View::Settings { .. } => "Settings".into(),
            View::Handbook { .. } => "Handbook".into(),
            View::Pager { title, .. } => sentence_case(title),
            View::Help { .. } => "Help".into(),
            View::Files { pick: Some(pk), .. } => sentence_case(pk.title()),
            View::Files { .. } => "Find files".into(),
            View::Library { kind, .. } if *kind == resilience_core::packs::PackKind::Map => {
                "Maps on this computer & USB sticks".into()
            }
            View::Library { .. } => "Offline library".into(),
            View::Chats { .. } => "Local messages".into(),
            View::Nearby { .. } => "People nearby".into(),
            View::Chat { peer, name, .. } => name.display(peer),
            _ => String::new(),
        }
    }

    fn home_view(&self, p: &Palette) -> Element<'_, Message> {
        let cats = self.app.home_categories();
        let tiles: Vec<Element<'_, Message>> = cats
            .iter()
            .enumerate()
            .map(|(i, c)| self.tile(i, &section_name(*c), category_description(*c), icon(*c), p))
            .collect();
        grid(tiles, self.columns())
    }

    fn category_view(&self, cat: Category, p: &Palette) -> Element<'_, Message> {
        let entries = self.app.entries(cat);
        if entries.is_empty() {
            return centered_note("Nothing is available here for your account.");
        }
        let tiles: Vec<Element<'_, Message>> = entries
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let name = self.app.entry_name(*e);
                let mut desc = self.entry_description(*e);
                if !self.app.entry_ready(*e) {
                    desc = format!("Not available on this computer. {desc}");
                }
                self.tile(i, &name, &desc, "", p)
            })
            .collect();
        grid(tiles, self.columns())
    }

    fn search_view(&self, query: &str, hits: &[SearchHit], p: &Palette) -> Element<'_, Message> {
        if hits.is_empty() {
            return centered_note(&format!("Nothing found for \u{201c}{query}\u{201d}."));
        }
        let sel = self.app.selected_row();
        let rows: Vec<Element<'_, Message>> = hits
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let (kind, name) = match h {
                    SearchHit::Category(c) => ("Section", section_name(*c)),
                    SearchHit::Entry(e) => ("App", self.app.entry_name(*e)),
                    SearchHit::Page(pg) => ("Guide", pg.title.clone()),
                };
                button(
                    row![
                        text(kind)
                            .size(13)
                            .width(Length::Fixed(80.0))
                            .style(style::dim_text),
                        text(name).size(17),
                    ]
                    .align_y(iced::Center),
                )
                .on_press(Message::Choose(i))
                .width(Fill)
                .padding([14, 20])
                .style(move |t, st| style::row_button(t, st, sel == Some(i)))
                .into()
            })
            .collect();
        let _ = p;
        scrollable(column(rows).spacing(6).padding([8, 28]))
            .height(Fill)
            .into()
    }

    /// Rows as in the shell: the themes, then the keyboard layouts.
    fn settings_view(&self, p: &Palette) -> Element<'_, Message> {
        let p = *p;
        let sel = self.app.selected_row();
        let themes = ThemeName::ALL;
        let swatches: Vec<Element<'_, Message>> = themes
            .iter()
            .enumerate()
            .map(|(i, th)| {
                let current = *th == self.app.settings.theme;
                let dot = Palette::of(*th).accent;
                let name = match th {
                    ThemeName::Green => "Green",
                    ThemeName::Amber => "Amber",
                    ThemeName::Cyan => "Blue",
                    ThemeName::Mono => "Grey",
                };
                let label = if current {
                    format!("{name}  \u{2713}")
                } else {
                    name.to_string()
                };
                button(
                    row![text("\u{25cf}").size(22).color(dot), text(label).size(16)]
                        .spacing(12)
                        .align_y(iced::Center),
                )
                .on_press(Message::Choose(i))
                .width(Fill)
                .padding([14, 18])
                .style(move |t, st| style::row_button(t, st, sel == Some(i)))
                .into()
            })
            .collect();
        let kb = &self.app.sys.keyboard;
        let now = kb.effective(self.app.settings.keyboard.as_deref());
        let layouts: Vec<Element<'_, Message>> = kb
            .layouts
            .iter()
            .enumerate()
            .map(|(j, l)| {
                let i = themes.len() + j;
                let label = if l.id == now {
                    format!("{}  \u{2713}", l.label)
                } else {
                    l.label.clone()
                };
                button(text(label).size(16))
                    .on_press(Message::Choose(i))
                    .width(Fill)
                    .padding([14, 18])
                    .style(move |t, st| style::row_button(t, st, sel == Some(i)))
                    .into()
            })
            .collect();
        let mode = self.app.mode();
        let account = column![
            text("Your account").size(18).font(style::UI_FONT_BOLD),
            text(format!("Security mode: {}", mode_name(mode)))
                .size(15)
                .color(p.accent),
            text(mode.description().to_string()).size(14),
            text(
                "The security mode comes from your account, not from a setting. \
                 An administrator can change it (handbook: \u{201c}Security modes\u{201d})."
            )
            .size(13)
            .style(style::dim_text),
        ]
        .spacing(8);
        let heading = |s: &'static str| text(s).size(13).style(style::dim_text);
        let left = column![heading("COLOUR"), row(swatches).spacing(10)]
            .push(Space::new().height(12))
            .push(heading("KEYBOARD LAYOUT"))
            .push(column(layouts).spacing(6))
            .spacing(10)
            .width(Length::FillPortion(3));
        let right = container(account)
            .padding(22)
            .width(Length::FillPortion(2))
            .style(move |_| style::card(&p));
        scrollable(row![left, right].spacing(24).padding([8, 28]))
            .height(Fill)
            .into()
    }

    fn handbook_view<'a>(
        &self,
        pages: &'a [resilience_core::handbook::Page],
        error: Option<&str>,
    ) -> Element<'a, Message> {
        if let Some(e) = error {
            return centered_note(&format!("The handbook could not be opened: {e}"));
        }
        let sel = self.app.selected_row();
        let rows: Vec<Element<'a, Message>> = pages
            .iter()
            .enumerate()
            .map(|(i, pg)| {
                button(text(pg.title.as_str()).size(17))
                    .on_press(Message::Choose(i))
                    .width(Fill)
                    .padding([14, 20])
                    .style(move |t, st| style::row_button(t, st, sel == Some(i)))
                    .into()
            })
            .collect();
        scrollable(
            container(column(rows).spacing(6).max_width(760.0))
                .center_x(Fill)
                .padding([8, 28]),
        )
        .height(Fill)
        .into()
    }

    /// Lines of a text page that fit the window.
    fn pager_rows(&self) -> usize {
        ((self.size.height - 180.0) / READ_LINE).floor().max(5.0) as usize
    }

    fn clamp_scroll(&mut self) {
        let rows = self.pager_rows();
        let total = match self.app.view() {
            View::Pager { text, .. } => text.lines().count(),
            View::Help { topic, .. } => help_text(*topic, self.app.mode()).lines().count(),
            _ => return,
        };
        if let Some(sc) = self.app.scroll_mut() {
            *sc = (*sc).min(total.saturating_sub(rows));
        }
    }

    /// A guide or report: the lines from the shell's scroll position, in a
    /// centred reading column; headings (lines in capitals) stand out.
    fn pager_view<'a>(&self, body: String, scroll: usize, p: &Palette) -> Element<'a, Message> {
        let rows = self.pager_rows();
        let total = body.lines().count();
        let lines: Vec<Element<'a, Message>> = body
            .lines()
            .skip(scroll)
            .take(rows)
            .map(|l| {
                if let Some(title) = l.strip_prefix("# ") {
                    return text(title.to_string())
                        .size(26)
                        .font(style::UI_FONT_BOLD)
                        .line_height(iced::widget::text::LineHeight::Absolute(
                            (READ_LINE * 2.0).into(),
                        ))
                        .into();
                }
                let heading = is_heading(l);
                let mut t = text(if l.is_empty() {
                    " ".to_string()
                } else {
                    l.to_string()
                })
                .size(READ_SIZE)
                .line_height(iced::widget::text::LineHeight::Absolute(READ_LINE.into()))
                .font(style::MONO_FONT);
                if heading {
                    t = t.color(p.accent).font(iced::Font {
                        weight: iced::font::Weight::Bold,
                        ..style::MONO_FONT
                    });
                }
                t.into()
            })
            .collect();
        let pos = if total > rows {
            format!(
                "{}\u{2013}{} of {} lines  \u{00b7}  wheel, arrows or PgUp/PgDn to scroll",
                scroll + 1,
                (scroll + rows).min(total),
                total
            )
        } else {
            String::new()
        };
        let page = column![
            column(lines),
            Space::new().height(Fill),
            text(pos).size(13).style(style::dim_text)
        ]
        .max_width(880.0);
        iced::widget::mouse_area(container(page).center_x(Fill).padding([8, 28]).height(Fill))
            .on_scroll(|d| {
                let dy = match d {
                    iced::mouse::ScrollDelta::Lines { y, .. } => y * 3.0,
                    iced::mouse::ScrollDelta::Pixels { y, .. } => y / READ_LINE,
                };
                Message::Wheel(-(dy.round() as i32))
            })
            .into()
    }

    /// Find files: what is typed goes to the shell (as in the TUI) and shows
    /// in the big field; results are rows (name, folder, size, date).
    fn files_view<'a>(
        &self,
        idx: &'a resilience_core::files::Index,
        query: &str,
        picking: bool,
        p: &Palette,
    ) -> Element<'a, Message> {
        let p = *p;
        let cursor = if (self.app.tick / 2).is_multiple_of(2) {
            "\u{258f}"
        } else {
            " "
        };
        let shown = if query.is_empty() {
            text(format!("Type part of a file name\u{2026}{cursor}"))
                .size(18)
                .style(style::dim_text)
        } else {
            text(format!("{query}{cursor}")).size(18)
        };
        let field = container(shown)
            .padding([12, 18])
            .width(Fill)
            .style(move |_| style::field(&p));
        let hits = resilience_core::files::search(idx, query);
        let sel = self.app.selected_row();
        let note = if query.trim().is_empty() {
            format!(
                "{} files in your home folder and open USB sticks can be searched.",
                idx.entries.len()
            )
        } else if hits.is_empty() {
            "No file name matches.".to_string()
        } else {
            format!("{} found", hits.len())
        };
        let rows: Vec<Element<'a, Message>> = hits
            .iter()
            .take(200)
            .enumerate()
            .map(|(i, e)| {
                let (folder, name) = match e.shown.rsplit_once('/') {
                    Some((d, n)) => (d.to_string(), n.to_string()),
                    None => (String::new(), e.shown.clone()),
                };
                let date = e
                    .modified
                    .map(resilience_shell::fmt::date_time)
                    .unwrap_or_default();
                button(
                    row![
                        text(file_glyph(&name))
                            .size(20)
                            .color(p.accent)
                            .width(Length::Fixed(28.0)),
                        column![
                            text(name).size(16),
                            text(folder).size(13).style(style::dim_text)
                        ]
                        .width(Fill),
                        text(resilience_core::human_bytes(e.size))
                            .size(14)
                            .style(style::dim_text)
                            .width(Length::Fixed(90.0)),
                        text(date)
                            .size(14)
                            .style(style::dim_text)
                            .width(Length::Fixed(140.0)),
                    ]
                    .spacing(12)
                    .align_y(iced::Center),
                )
                .on_press(Message::Choose(i))
                .width(Fill)
                .padding([10, 18])
                .style(move |t, st| style::row_button(t, st, sel == Some(i)))
                .into()
            })
            .collect();
        let hint = if picking {
            "Enter or click: choose this file   \u{00b7}   Esc: cancel"
        } else {
            "Enter or click: view   \u{00b7}   F3: details & checksum   \u{00b7}   F2: search again"
        };
        column![
            field,
            row![
                text(note).size(13).style(style::dim_text),
                Space::new().width(Fill),
                text(hint).size(13).style(style::dim_text)
            ],
            scrollable(column(rows).spacing(4)).height(Fill),
        ]
        .spacing(10)
        .padding([8, 28])
        .into()
    }

    fn library_view<'a>(
        &self,
        kind: resilience_core::packs::PackKind,
        items: &'a [resilience_shell::app::LibItem],
        p: &Palette,
    ) -> Element<'a, Message> {
        let map = kind == resilience_core::packs::PackKind::Map;
        let hint = if map {
            "Enter or click: open the map   \u{00b7}   c: check   \u{00b7}   u: USB stick"
        } else {
            "Enter or click: read   \u{00b7}   s: search   \u{00b7}   c: check   \u{00b7}   u: USB stick   \u{00b7}   p: get packs"
        };
        if items.is_empty() {
            return column![
                centered_note(if map {
                    "No map yet. Maps come as packs: see Resource packs, or a USB stick (u)."
                } else {
                    "No library yet. Press p to get packs, or u to read one from a USB stick."
                }),
                text(hint).size(13).style(style::dim_text)
            ]
            .padding([8, 28])
            .into();
        }
        let tiles: Vec<Element<'_, Message>> = items
            .iter()
            .enumerate()
            .map(|(i, it)| {
                let desc = format!(
                    "{}\n{} \u{00b7} {}",
                    it.description,
                    it.tag(),
                    resilience_core::human_bytes(it.size)
                );
                self.tile(
                    i,
                    &it.title,
                    &desc,
                    if map { "\u{25ce}" } else { "\u{25a4}" },
                    p,
                )
            })
            .collect();
        column![
            row![
                Space::new().width(Fill),
                text(hint).size(13).style(style::dim_text)
            ]
            .padding([0, 28]),
            grid(tiles, self.columns())
        ]
        .spacing(6)
        .into()
    }

    fn chats_view<'a>(
        &self,
        rows: &'a [resilience_shell::app::ChatRow],
        status: &'a messages::Status,
        p: &Palette,
    ) -> Element<'a, Message> {
        let p = *p;
        let now = resilience_shell::app::chat_now();
        let sel = self.app.selected_row();
        let (state, ok) = match status {
            messages::Status::Running { address, store } => (
                format!(
                    "Messaging is on. My address: {} (m shows all of it). {}",
                    messages::short_address(address),
                    if *store {
                        "People who are away can be written to: a store keeps the message."
                    } else {
                        "No message store: both computers must be on at the same time."
                    }
                ),
                true,
            ),
            messages::Status::Stopped { .. } => (
                "Messaging is not running for your account: nothing is sent or received. \
                 Log out and in again."
                    .to_string(),
                false,
            ),
        };
        let banner = container(
            row![
                text(if ok { "\u{25cf}" } else { "\u{25b2}" })
                    .size(16)
                    .color(if ok { p.accent } else { p.warn }),
                text(state).size(14)
            ]
            .spacing(10)
            .align_y(iced::Center),
        )
        .padding([12, 18])
        .width(Fill)
        .style(move |_| style::card(&p));
        let hint = "Enter or click: open   \u{00b7}   n: write to an address   \u{00b7}   p: people nearby   \u{00b7}   m: my address";
        let list: Element<'a, Message> = if rows.is_empty() {
            centered_note(
                "No messages yet. Press p to see the people on your local network, \
                 or n to write to an address someone gave you.",
            )
        } else {
            let items: Vec<Element<'a, Message>> = rows
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let c = &r.conv;
                    let who = if c.last_direction == messages::Direction::Out {
                        "You: "
                    } else {
                        ""
                    };
                    let flag = if c.failed > 0 {
                        text("failed").size(13).color(p.error)
                    } else if c.pending > 0 {
                        text("waiting").size(13).style(style::dim_text)
                    } else {
                        text("")
                    };
                    let preview = format!("{who}{}", messages::clean_line(&c.last_text, 120));
                    button(
                        row![
                            text(initial(&r.name.display(&c.peer)))
                                .size(18)
                                .font(style::UI_FONT_BOLD)
                                .color(p.accent)
                                .width(Length::Fixed(32.0)),
                            column![
                                text(r.name.display(&c.peer))
                                    .size(16)
                                    .font(style::UI_FONT_BOLD),
                                text(preview).size(14).style(style::dim_text)
                            ]
                            .width(Fill),
                            flag,
                            text(messages::ago(c.last_time, now))
                                .size(13)
                                .style(style::dim_text)
                                .width(Length::Fixed(110.0)),
                        ]
                        .spacing(12)
                        .align_y(iced::Center),
                    )
                    .on_press(Message::Choose(i))
                    .width(Fill)
                    .padding([12, 18])
                    .style(move |t, st| style::row_button(t, st, sel == Some(i)))
                    .into()
                })
                .collect();
            scrollable(column(items).spacing(4)).height(Fill).into()
        };
        let claimed = sel
            .and_then(|i| rows.get(i))
            .is_some_and(|r| matches!(r.name, messages::Name::Claimed(_)));
        let mut c = column![
            banner,
            row![
                Space::new().width(Fill),
                text(hint).size(13).style(style::dim_text)
            ],
            list
        ]
        .spacing(10)
        .padding([8, 28]);
        if claimed {
            c = c.push(
                text("A name in quotes with ? is what that address says about itself: anyone can claim any name.")
                    .size(13)
                    .style(style::dim_text),
            );
        }
        c.into()
    }

    fn nearby_view<'a>(
        &self,
        rows: &'a [resilience_shell::app::NearbyRow],
        p: &Palette,
    ) -> Element<'a, Message> {
        let p = *p;
        let now = resilience_shell::app::chat_now();
        let sel = self.app.selected_row();
        let hint = "Enter or click: write   \u{00b7}   a: name this person   \u{00b7}   i: address   \u{00b7}   r: refresh";
        let list: Element<'a, Message> = if rows.is_empty() {
            centered_note(
                "Nobody has been heard on the local network yet. Other computers appear \
                 here a little after they join the same network.",
            )
        } else {
            let items: Vec<Element<'a, Message>> = rows
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    button(
                        row![
                            text(initial(&r.name.display(&r.peer.address)))
                                .size(18)
                                .font(style::UI_FONT_BOLD)
                                .color(p.accent)
                                .width(Length::Fixed(32.0)),
                            column![
                                text(r.name.display(&r.peer.address))
                                    .size(16)
                                    .font(style::UI_FONT_BOLD),
                                text(messages::format_address(&r.peer.address))
                                    .size(13)
                                    .font(style::MONO_FONT)
                                    .style(style::dim_text)
                            ]
                            .width(Fill),
                            text(format!("heard {}", messages::ago(r.peer.heard, now)))
                                .size(13)
                                .style(style::dim_text),
                        ]
                        .spacing(12)
                        .align_y(iced::Center),
                    )
                    .on_press(Message::Choose(i))
                    .width(Fill)
                    .padding([12, 18])
                    .style(move |t, st| style::row_button(t, st, sel == Some(i)))
                    .into()
                })
                .collect();
            scrollable(column(items).spacing(4)).height(Fill).into()
        };
        column![
            row![
                Space::new().width(Fill),
                text(hint).size(13).style(style::dim_text)
            ],
            list,
            text(
                "A name in quotes with ? is only what that address says about itself. Compare the \
                  address with the person (in person or by voice), then press a to name it."
            )
            .size(13)
            .style(style::dim_text),
        ]
        .spacing(10)
        .padding([8, 28])
        .into()
    }

    fn chat_view<'a>(&self, records: &'a [messages::Record], p: &Palette) -> Element<'a, Message> {
        let p = *p;
        let now = resilience_shell::app::chat_now();
        let sel = self.app.selected_row();
        let hint = "w: write   \u{00b7}   f: send a file   \u{00b7}   o: open   \u{00b7}   s: save   \u{00b7}   c: cancel   \u{00b7}   a: name   \u{00b7}   i: address";
        let list: Element<'a, Message> = if records.is_empty() {
            centered_note("No messages with this person yet. Press w to write one.")
        } else {
            let items: Vec<Element<'a, Message>> = records
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let mine = r.direction == messages::Direction::Out;
                    let color = match (r.direction, r.state) {
                        (messages::Direction::In, _) if r.verified => p.accent,
                        (messages::Direction::In, _) => p.warn,
                        (_, messages::State::Failed) => p.error,
                        _ => p.dim,
                    };
                    let mut head =
                        format!("{} \u{00b7} {}", r.marker(), messages::ago(r.created, now));
                    if !r.detail.is_empty() && r.state != messages::State::Delivered {
                        head.push_str(&format!(
                            " \u{00b7} {}",
                            messages::clean_line(&r.detail, 80)
                        ));
                    }
                    let mut b = column![
                        text(messages::clean_text(&r.text, 200).join("\n")).size(15),
                        text(head).size(12).color(color),
                    ]
                    .spacing(4);
                    if let Some(file) = &r.file {
                        let mut l = format!(
                            "File: {} ({})",
                            file.shown(),
                            messages::size_text(file.size)
                        );
                        if !mine {
                            l.push_str(if file.data.is_some() {
                                "  \u{00b7}  o open, s save"
                            } else {
                                "  \u{00b7}  not kept"
                            });
                        }
                        b = b.push(text(l).size(13).color(p.accent));
                    }
                    let bubble = button(b)
                        .on_press(Message::Choose(i))
                        .padding([10, 16])
                        .width(Length::FillPortion(3))
                        .style(move |t, st| style::row_button(t, st, sel == Some(i)));
                    let r: Element<'a, Message> = if mine {
                        row![Space::new().width(Length::FillPortion(1)), bubble].into()
                    } else {
                        row![bubble, Space::new().width(Length::FillPortion(1))].into()
                    };
                    r
                })
                .collect();
            scrollable(column(items).spacing(8))
                .anchor_bottom()
                .height(Fill)
                .into()
        };
        column![
            row![
                Space::new().width(Fill),
                text(hint).size(13).style(style::dim_text)
            ],
            list
        ]
        .spacing(10)
        .padding([8, 28])
        .into()
    }

    fn entry_description(&self, e: Entry) -> String {
        match e {
            Entry::App(i) => self.app.catalog.apps[i].description.clone(),
            Entry::Builtin(b) => b.description().to_string(),
        }
    }

    fn tile<'a>(
        &self,
        i: usize,
        title: &str,
        desc: &str,
        glyph: &'static str,
        p: &Palette,
    ) -> Element<'a, Message> {
        let mut c = column![].spacing(8);
        if !glyph.is_empty() {
            c = c.push(text(glyph).size(28).color(p.accent));
        }
        c = c
            .push(text(title.to_string()).size(18).font(style::UI_FONT_BOLD))
            .push(text(desc.to_string()).size(14).style(style::dim_text));
        let selected = self.app.selected_row() == Some(i);
        button(c)
            .on_press(Message::Choose(i))
            .width(Fill)
            .height(Length::Fixed(168.0))
            .padding(22)
            .style(move |t, st| style::tile(t, st, selected))
            .into()
    }

    fn columns(&self) -> usize {
        ((self.size.width - 56.0) / 300.0).floor().clamp(1.0, 4.0) as usize
    }

    fn text_view(&self, p: &Palette) -> Element<'_, Message> {
        container(textpanel::view(&self.screen, p))
            .padding([4, 28])
            .height(Fill)
            .into()
    }

    fn running_view(&self, label: &str) -> Element<'_, Message> {
        centered_note(&format!(
            "{label} is open in its own window. Close it to come back here."
        ))
    }

    fn status_bar(&self, p: &Palette) -> Element<'_, Message> {
        let p = *p;
        let s = style::status_line(&self.app);
        let toast = self
            .app
            .current_toast()
            .map(|n| n.text.clone())
            .unwrap_or_default();
        container(
            row![
                text(toast).size(14),
                Space::new().width(Fill),
                text(s).size(14).style(style::dim_text),
            ]
            .spacing(16),
        )
        .padding([10, 28])
        .style(move |_| style::status_bar(&p))
        .into()
    }
}

fn grid<'a>(tiles: Vec<Element<'a, Message>>, cols: usize) -> Element<'a, Message> {
    let mut rows = column![].spacing(16);
    let mut it = tiles.into_iter().peekable();
    while it.peek().is_some() {
        let mut r = row![].spacing(16);
        for _ in 0..cols {
            match it.next() {
                Some(t) => r = r.push(t),
                None => r = r.push(Space::new().width(Fill)),
            }
        }
        rows = rows.push(r);
    }
    scrollable(container(rows).padding([8, 28]))
        .height(Fill)
        .into()
}

/// The first letter of a name, for its round mark in the lists.
fn initial(name: &str) -> String {
    name.chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_else(|| "?".into())
}

fn centered_note<'a>(s: &str) -> Element<'a, Message> {
    container(text(s.to_string()).size(18).style(style::dim_text))
        .center(Fill)
        .into()
}

fn mode_name(m: resilience_core::mode::Mode) -> &'static str {
    use resilience_core::mode::Mode;
    match m {
        Mode::UltraStable => "Ultra-Stable",
        Mode::Stable => "Stable",
        Mode::Advanced => "Advanced",
    }
}

/// The shell's help for the screen, after a note on the pointer (the keys
/// it lists work the same here).
fn help_text(topic: resilience_shell::app::HelpTopic, mode: resilience_core::mode::Mode) -> String {
    let mut out = String::from(
        "== IN THE GRAPHICAL INTERFACE ==\n  \
         Click a tile or a row to open it. Back and Home are at the top left;\n  \
         the search box is at the top right. Every key below works too.\n\n",
    );
    out.push_str(&resilience_shell::help::lines(topic, mode).join("\n"));
    out
}

/// Height of the top bar, the same on every screen.
const BAR_H: f32 = 76.0;

/// A plain glyph by file type (no icon font).
fn file_glyph(name: &str) -> &'static str {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "txt" | "md" | "csv" | "tsv" | "log" | "toml" | "json" => "\u{2261}",
        "pdf" | "epub" | "zim" => "\u{25a4}",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" => "\u{25a3}",
        "zip" | "tar" | "gz" | "zst" | "xz" | "7z" => "\u{25a6}",
        "age" | "gpg" | "asc" | "minisig" | "sig" => "\u{25c8}",
        "mbtiles" => "\u{25ce}",
        _ => "\u{25a1}",
    }
}

/// Reading view: mono (guides are laid out in columns), comfortable size.
const READ_SIZE: f32 = 15.0;
const READ_LINE: f32 = 24.0;

/// A handbook heading: a line in capitals, not indented ("MOVING AROUND").
fn is_heading(l: &str) -> bool {
    (l.starts_with("== ") && l.ends_with(" ==")) || is_caps_line(l)
}

fn is_caps_line(l: &str) -> bool {
    !l.starts_with(' ')
        && l.chars().any(|c| c.is_alphabetic())
        && !l.chars().any(|c| c.is_lowercase())
}

/// "KNOWLEDGE & DOCUMENTATION" -> "Knowledge & documentation"; a title
/// with lower-case letters is kept as it is.
fn sentence_case(s: &str) -> String {
    if s.chars().any(|c| c.is_lowercase()) {
        return s.to_string();
    }
    let t = s.to_lowercase();
    let mut ch = t.chars();
    match ch.next() {
        Some(f) => f.to_uppercase().chain(ch).collect(),
        None => t,
    }
}

fn section_name(c: Category) -> String {
    sentence_case(c.title())
}

/// One plain glyph per section (the bundled font has them; no icon font).
fn icon(c: Category) -> &'static str {
    match c {
        Category::Knowledge => "\u{25a4}",
        Category::Maps => "\u{25ce}",
        Category::Communications => "\u{25c8}",
        Category::Networking => "\u{25cd}",
        Category::Radio => "\u{25c9}",
        Category::Security => "\u{25a3}",
        Category::Productivity => "\u{25a7}",
        Category::Entertainment => "\u{25c6}",
        Category::System => "\u{25a6}",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_and_headings() {
        assert_eq!(sentence_case("SYSTEM MANAGEMENT"), "System management");
        assert_eq!(sentence_case("First steps"), "First steps");
        assert!(is_heading("MOVING AROUND"));
        assert!(!is_heading("  F1   help"));
        assert!(!is_heading("Welcome to Resilience"));
        assert!(!is_heading(""));
    }
}
