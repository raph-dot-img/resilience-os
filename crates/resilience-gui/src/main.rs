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
use resilience_shell::app::{
    App, Effect, Entry, Modal, PackRow, PacksScreen, SearchHit, Startup, View, category_description,
};
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
    /// A dialog button: the key it stands for.
    Press(KeyEvent),
}

struct Gui {
    app: App,
    size: iced::Size,
    /// Label of the program running in its terminal window.
    running: Option<String>,
    /// The terminal renderer's output for screens without a native view.
    screen: textpanel::Screen,
    /// Rows in the list drawn last (see [`LIST`]): lets the keyboard
    /// selection scroll into view.
    rows_shown: std::cell::Cell<usize>,
}

/// The id of the scrollable row list on native list screens.
const LIST: &str = "rows";

impl Gui {
    fn new() -> (Gui, Task<Message>) {
        let mut app = App::new(Startup::load());
        app.apply_saved_layout();
        // Development previews only (scripts/gui-preview.sh): a sample catalog.
        if cfg!(debug_assertions)
            && let Some(p) = std::env::var_os("RESILIENCE_PACKS_CONFIG")
        {
            app.paths.packs_config = p.into();
        }
        let gui = Gui {
            app,
            size: iced::Size::new(1280.0, 800.0),
            running: None,
            screen: textpanel::Screen::default(),
            rows_shown: std::cell::Cell::new(0),
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
        let key = matches!(m, Message::Key(_));
        let (depth, before) = (self.app.stack.len(), self.app.selected_row());
        let mut task = self.step(m);
        // Arrow keys move the selection: keep it on screen. Snapping to the
        // same fraction of the list always shows that row.
        let after = self.app.selected_row();
        if key
            && self.app.stack.len() == depth
            && after != before
            && let Some(i) = after
        {
            {
                let n = self.rows_shown.get().max(2);
                let mut y = (i as f32 / (n - 1) as f32).clamp(0.0, 1.0);
                // A conversation is anchored at its newest (last) message.
                if matches!(self.app.view(), View::Chat { .. }) {
                    y = 1.0 - y;
                }
                task = Task::batch([
                    task,
                    iced::widget::operation::snap_to(
                        iced::widget::Id::new(LIST),
                        iced::widget::scrollable::RelativeOffset { x: 0.0, y },
                    ),
                ]);
            }
        }
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
            && ((self.app.modal.is_some() && !self.native_modal())
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
                        | View::Wifi { .. }
                        | View::Recovery { .. }
                        | View::Users { .. }
                        | View::Media { .. }
                        | View::Generations { .. }
                        | View::Notes { .. }
                        | View::Updates { .. }
                        | View::Backups { .. }
                        | View::Community { .. }
                        | View::PeerMenu { .. }
                        | View::ChooseMode { .. }
                        | View::Choices { .. }
                        | View::Packs { .. }
                        | View::Keys { .. }
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
                } else if self.app.modal.is_none() && matches!(self.app.view(), View::Packs { .. })
                {
                    // A click ticks the pack (Enter still shows its details).
                    if let View::Packs { s, .. } = self.app.view()
                        && !s.rows().get(i).is_some_and(|r| r.selectable())
                    {
                        return Task::none();
                    }
                    if let Some(l) = self.app.list_mut() {
                        l.select(Some(i));
                    }
                    self.app
                        .handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE))
                } else {
                    self.app.choose_row(i)
                }
            }
            Message::Press(k) => {
                if self.running.is_some() {
                    return Task::none();
                }
                self.app.handle_key(k)
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
                View::Recovery { .. } => self.recovery_view(&p),
                View::Users { rows, error, .. } => self.users_view(rows, error.as_deref(), &p),
                View::Media { volumes, error, .. } => {
                    self.media_view(volumes, error.as_deref(), &p)
                }
                View::Generations { gens, .. } => self.generations_view(gens, &p),
                View::Notes { notes, .. } => self.notes_view(notes, &p),
                View::Updates { rows, error, .. } => self.updates_view(rows, error.as_deref(), &p),
                View::Backups { files, .. } => self.backups_view(files, &p),
                View::Community { rows, .. } => self.community_view(rows, &p),
                View::PeerMenu {
                    name,
                    peer,
                    actions,
                    ..
                } => self.peer_menu_view(name, peer, actions, &p),
                View::ChooseMode { row, .. } => self.choose_mode_view(row, &p),
                View::Choices { items, .. } => self.choices_view(items, &p),
                View::Packs { s, .. } => self.packs_view(s, &p),
                View::Keys { rows, .. } => self.keys_view(rows, &p),
                View::Wifi {
                    overview, error, ..
                } => self.wifi_view(overview.as_ref(), error.as_deref(), &p),
                View::Help { topic, scroll } => {
                    self.pager_view(help_text(*topic, self.app.mode()), *scroll, &p)
                }
                _ => self.text_view(&p),
            }
        };
        let body: Element<'_, Message> = match self.dialog_view(&p) {
            Some(d) if self.running.is_none() => iced::widget::stack![body, d].into(),
            _ => body,
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
            View::Wifi { .. } => "Wi-Fi".into(),
            View::Recovery { .. } => "Recovery & diagnostics".into(),
            View::Users { .. } => "Users & security modes".into(),
            View::Generations { .. } => "System generations".into(),
            View::Notes { .. } => "Notes".into(),
            View::Community { .. } => "Community network".into(),
            View::PeerMenu { name, .. } => format!("\"{}\"?", resilience_core::printable(name)),
            View::ChooseMode { row, .. } => format!("New security mode for {}", row.name),
            View::Choices { title, .. } => sentence_case(title),
            View::Packs { .. } => "Resource packs".into(),
            View::Keys { .. } => "Keys & contacts".into(),
            View::Updates { .. } => "Update from a USB stick".into(),
            View::Backups { volume, .. } => format!("Backups on {}", volume.display_name()),
            View::Media { purpose, .. } => match purpose {
                resilience_shell::app::MediaPurpose::InstallPacks => "Copy packs from a USB stick",
                resilience_shell::app::MediaPurpose::ReadPacks => "Read packs on a USB stick",
                resilience_shell::app::MediaPurpose::Backups => "Backups & USB sticks",
                resilience_shell::app::MediaPurpose::ExportKeys => {
                    "Save your public keys on a USB stick"
                }
            }
            .into(),
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
        self.rows_shown.set(rows.len());
        scrollable(column(rows).spacing(6).padding([8, 28]))
            .id(iced::widget::Id::new(LIST))
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
        self.rows_shown.set(rows.len());
        scrollable(
            container(column(rows).spacing(6).max_width(760.0))
                .center_x(Fill)
                .padding([8, 28]),
        )
        .id(iced::widget::Id::new(LIST))
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
            {
                self.rows_shown.set(rows.len());
                scrollable(column(rows).spacing(4))
                    .id(iced::widget::Id::new(LIST))
                    .spacing(8)
                    .height(Fill)
            },
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
            self.rows_shown.set(items.len());
            scrollable(column(items).spacing(4))
                .id(iced::widget::Id::new(LIST))
                .spacing(8)
                .height(Fill)
                .into()
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
            self.rows_shown.set(items.len());
            scrollable(column(items).spacing(4))
                .id(iced::widget::Id::new(LIST))
                .spacing(8)
                .height(Fill)
                .into()
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
            self.rows_shown.set(items.len());
            scrollable(column(items).spacing(8))
                .id(iced::widget::Id::new(LIST))
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

    /// A native list of rows: the shared look of the simple menus.
    fn rows_view<'a>(
        &self,
        rows: Vec<RowSpec>,
        hint: &'static str,
        empty: &str,
        footer: String,
        p: &Palette,
    ) -> Element<'a, Message> {
        let p = *p;
        let sel = self.app.selected_row();
        let list: Element<'a, Message> = if rows.is_empty() {
            centered_note(empty)
        } else {
            let items: Vec<Element<'a, Message>> = rows
                .into_iter()
                .enumerate()
                .map(|(i, r)| {
                    if r.header {
                        return container(
                            text(r.title)
                                .size(13)
                                .font(style::UI_FONT_BOLD)
                                .color(p.accent),
                        )
                        .padding(iced::Padding::from([0, 18]).top(14).bottom(2))
                        .into();
                    }
                    let mut info = column![text(r.title).size(16).font(style::UI_FONT_BOLD)];
                    if !r.sub.is_empty() {
                        info = info.push(text(r.sub).size(13).color(if r.warn {
                            p.warn
                        } else {
                            p.dim
                        }));
                    }
                    button(
                        row![
                            text(r.glyph)
                                .size(20)
                                .color(p.accent)
                                .width(Length::Fixed(32.0)),
                            info.width(Fill),
                            text(r.right)
                                .size(13)
                                .color(if r.mark { p.accent } else { p.dim }),
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
            self.rows_shown.set(items.len());
            scrollable(column(items).spacing(4))
                .id(iced::widget::Id::new(LIST))
                .spacing(8)
                .height(Fill)
                .into()
        };
        let mut c = column![
            row![
                Space::new().width(Fill),
                text(hint).size(13).style(style::dim_text)
            ],
            list
        ]
        .spacing(10)
        .padding([8, 28]);
        if !footer.is_empty() {
            c = c.push(text(footer).size(13).style(style::dim_text));
        }
        c.into()
    }

    fn recovery_view<'a>(&self, p: &Palette) -> Element<'a, Message> {
        let rows = self
            .app
            .recovery_items()
            .iter()
            .enumerate()
            .map(|(i, r)| RowSpec {
                glyph: "\u{2692}",
                title: r.name().to_string(),
                sub: r.description().to_string(),
                right: format!("{}", i + 1),
                ..RowSpec::default()
            })
            .collect();
        self.rows_view(
            rows,
            "Enter, click or its number: open",
            "",
            "Other consoles always offer a text login: Ctrl+Alt+F2 (back here: Ctrl+Alt+F1)."
                .into(),
            p,
        )
    }

    fn users_view<'a>(
        &self,
        rows: &[resilience_core::admin::AccountRow],
        error: Option<&str>,
        p: &Palette,
    ) -> Element<'a, Message> {
        if let Some(e) = error {
            return centered_note(&format!(
                "The list of accounts could not be read: {e}. Security modes can still be \
                 changed by an administrator in the system configuration."
            ));
        }
        let rows = rows
            .iter()
            .map(|r| {
                let you = r.name == self.app.who.user;
                let mut sub = if r.description.is_empty() {
                    String::new()
                } else {
                    r.description.clone()
                };
                let pending = r.active != r.effective;
                if pending {
                    sub = format!(
                        "Active permissions are still {}; restart to apply.",
                        r.active
                    );
                }
                RowSpec {
                    glyph: "\u{25c9}",
                    title: if you {
                        format!("{} (you)", r.name)
                    } else {
                        r.name.clone()
                    },
                    sub,
                    right: r.effective.label().to_string(),
                    mark: you,
                    warn: pending,
                    header: false,
                }
            })
            .collect();
        self.rows_view(
            rows,
            "Enter or click: change the security mode   \u{00b7}   r: refresh",
            "No accounts are managed by Resilience on this system.",
            "Changing a mode needs an administrator's password.".into(),
            p,
        )
    }

    fn media_view<'a>(
        &self,
        volumes: &[resilience_core::media::Volume],
        error: Option<&str>,
        p: &Palette,
    ) -> Element<'a, Message> {
        if let Some(e) = error {
            return centered_note(&format!(
                "The list of USB sticks could not be read: {e}. Press r to try again."
            ));
        }
        let rows = volumes
            .iter()
            .map(|v| {
                let state = if !v.usable() {
                    "cannot be used".to_string()
                } else if let Some(m) = &v.mountpoint {
                    match v.available {
                        Some(a) => format!(
                            "open at {} \u{00b7} {} free",
                            m.display(),
                            resilience_core::human_bytes(a)
                        ),
                        None => format!("open at {}", m.display()),
                    }
                } else {
                    format!("{} \u{00b7} not open yet", v.fstype)
                };
                RowSpec {
                    glyph: "\u{25ad}",
                    title: v.display_name(),
                    sub: state,
                    right: resilience_core::human_bytes(v.size),
                    mark: v.mountpoint.is_some(),
                    warn: !v.usable(),
                    header: false,
                }
            })
            .collect();
        self.rows_view(
            rows,
            "Enter or click: open   \u{00b7}   e: eject   \u{00b7}   r: refresh",
            "No USB stick or memory card found. Plug one in, wait a few seconds, then press r.",
            "Eject (e) a stick before pulling it out.".into(),
            p,
        )
    }

    fn generations_view<'a>(
        &self,
        gens: &[resilience_core::recovery::Generation],
        p: &Palette,
    ) -> Element<'a, Message> {
        let rows = gens
            .iter()
            .map(|g| {
                let state = match (g.booted, g.running, g.default) {
                    (true, _, true) => "running, starts by default",
                    (true, _, false) => "running, not the default",
                    (false, true, _) => "active, not restarted",
                    (false, false, true) => "default at next start",
                    (false, false, false) => "in the boot menu",
                };
                RowSpec {
                    glyph: "\u{25f7}",
                    title: format!(
                        "Generation {} \u{00b7} {}",
                        g.number,
                        g.created
                            .map(resilience_shell::fmt::date_time)
                            .unwrap_or_else(|| "date unknown".into())
                    ),
                    sub: format!(
                        "{} \u{00b7} version {} \u{00b7} kernel {}",
                        state,
                        g.nixos_version.as_deref().unwrap_or("?"),
                        g.kernel.as_deref().unwrap_or("?")
                    ),
                    right: if g.default {
                        "default".into()
                    } else {
                        String::new()
                    },
                    mark: g.default,
                    warn: false,
                    header: false,
                }
            })
            .collect();
        self.rows_view(
            rows,
            "Enter or click: start this one at every boot   \u{00b7}   r: refresh",
            "No generations are recorded on this machine. This is normal on the live system, \
             which always starts the same way.",
            "Generations hold system software and settings only, never your personal files. \
             To try one once, choose it in the boot menu while the computer starts."
                .into(),
            p,
        )
    }

    fn notes_view<'a>(
        &self,
        notes: &[resilience_core::notes::Note],
        p: &Palette,
    ) -> Element<'a, Message> {
        let rows = notes
            .iter()
            .map(|n| RowSpec {
                glyph: "\u{270e}",
                title: n.name.clone(),
                sub: n
                    .modified
                    .map(resilience_shell::fmt::date_time)
                    .unwrap_or_default(),
                right: resilience_core::human_bytes(n.size),
                ..RowSpec::default()
            })
            .collect();
        self.rows_view(
            rows,
            "Enter or click: open   \u{00b7}   n: new note   \u{00b7}   r: refresh",
            "No notes yet. Press n to write your first one.",
            "Notes are plain text files in your Notes folder.".into(),
            p,
        )
    }

    fn updates_view<'a>(
        &self,
        rows: &[resilience_shell::app::UpdateRow],
        error: Option<&str>,
        p: &Palette,
    ) -> Element<'a, Message> {
        if let Some(e) = error {
            return centered_note(&format!(
                "The list of USB sticks could not be read: {e}. Press r to try again."
            ));
        }
        let running = self
            .app
            .update_settings()
            .map(|u| format!("This computer runs Resilience {} {}.", u.edition, u.version))
            .unwrap_or_default();
        let rows = rows
            .iter()
            .map(|r| {
                let (state, mark, warn) = match (&r.volume.mountpoint, &r.claim) {
                    _ if r.volume.problem.is_some() => ("cannot be used".to_string(), false, true),
                    (None, _) => ("not open yet: Enter opens it".to_string(), false, false),
                    (Some(_), Some(Ok(u))) => (format!("update to {}", u.version), true, false),
                    (Some(_), Some(Err(_))) => {
                        ("the update on it cannot be read".to_string(), false, true)
                    }
                    (Some(_), None) => ("no update on this stick".to_string(), false, false),
                };
                RowSpec {
                    glyph: "\u{21e7}",
                    title: r.volume.display_name(),
                    sub: state,
                    right: resilience_core::human_bytes(r.volume.size),
                    mark,
                    warn,
                    header: false,
                }
            })
            .collect();
        self.rows_view(
            rows,
            "Enter or click: open / install   \u{00b7}   c: check   \u{00b7}   r: refresh",
            "No USB stick or memory card found. Plug in the stick with the update, wait a \
             few seconds, then press r.",
            running,
            p,
        )
    }

    fn backups_view<'a>(
        &self,
        files: &[resilience_core::backup::BackupFile],
        p: &Palette,
    ) -> Element<'a, Message> {
        let mut rows = vec![RowSpec {
            glyph: "+",
            title: "Make a new backup of my files".into(),
            sub: "Your whole home folder in one encrypted file on this stick. You choose its \
                  passphrase."
                .into(),
            ..RowSpec::default()
        }];
        rows.extend(files.iter().map(|b| RowSpec {
            glyph: "\u{25a3}",
            title: resilience_core::backup::human_stamp(&b.stamp),
            sub: if b.has_checksum {
                "Enter restores it into a new folder; c checks it first".into()
            } else {
                "no checksum file: it cannot be checked".into()
            },
            right: resilience_core::human_bytes(b.size),
            warn: !b.has_checksum,
            ..RowSpec::default()
        }));
        self.rows_view(
            rows,
            "Enter or click: back up / restore   \u{00b7}   c: check   \u{00b7}   e: eject",
            "",
            "A restore never overwrites your files: it goes into a new folder in Restored.".into(),
            p,
        )
    }

    fn community_view<'a>(
        &self,
        rows: &[resilience_shell::app::CommunityRow],
        p: &Palette,
    ) -> Element<'a, Message> {
        use resilience_shell::app::CommunityRow as R;
        let now = resilience_shell::app::chat_now();
        let on = |title: String, sub: &str| RowSpec {
            glyph: "\u{25cf}",
            title,
            sub: sub.into(),
            right: "on".into(),
            mark: true,
            warn: false,
            header: false,
        };
        let heard = rows
            .iter()
            .filter(|r| matches!(r, R::Machine { .. }))
            .count();
        let rows = rows
            .iter()
            .map(|r| match r {
                R::Sharing(s) => on(
                    format!(
                        "Sharing {} on {} as \"{}\"",
                        s.services.join(", "),
                        s.interface,
                        s.name
                    ),
                    "Enter or click: stop sharing",
                ),
                R::Share => RowSpec {
                    glyph: "\u{21c4}",
                    title: "Share this computer's resources".into(),
                    sub:
                        "Packs, library, a folder or pages, on one network (administrator password)"
                            .into(),
                    ..RowSpec::default()
                },
                R::Address => RowSpec {
                    glyph: "\u{2328}",
                    title: "Type the address of a computer".into(),
                    sub: "When someone gives you their computer's address".into(),
                    ..RowSpec::default()
                },
                R::Hotspot { on: Some(what) } => {
                    on(format!("Hotspot {what}"), "Enter or click: turn it off")
                }
                R::Hotspot { on: None } => RowSpec {
                    glyph: "\u{25ce}",
                    title: "Create a network: hotspot".into(),
                    sub: "This computer becomes a Wi-Fi network others can join".into(),
                    ..RowSpec::default()
                },
                R::Mesh { on: true } => on("Mesh network".into(), "Enter or click: turn it off"),
                R::Mesh { on: false } => RowSpec {
                    glyph: "\u{2b21}",
                    title: "Create a network: mesh".into(),
                    sub: "Wi-Fi between computers, without a router".into(),
                    ..RowSpec::default()
                },
                R::Machine { heard, peer } => {
                    let what: Vec<&str> = heard.services.iter().map(|s| s.id()).collect();
                    RowSpec {
                        glyph: "\u{25a2}",
                        title: format!("\"{}\"?", resilience_core::printable(&heard.name)),
                        sub: format!(
                            "shares {} \u{00b7} {}",
                            what.join(", "),
                            match peer {
                                Some(p) => p.display(),
                                None => "not reachable from here".into(),
                            }
                        ),
                        right: format!("heard {}", messages::ago(heard.heard, now)),
                        warn: peer.is_none(),
                        ..RowSpec::default()
                    }
                }
            })
            .collect();
        let footer = if heard == 0 {
            "No computer sharing has been heard yet. They appear here a little after joining \
             the same network (Wi-Fi, cable or mesh)."
                .to_string()
        } else {
            "A name in quotes with ? is only what that computer says about itself. Packs you \
             get are checked; pages and files are as trustworthy as whoever runs it."
                .to_string()
        };
        self.rows_view(
            rows,
            "Enter or click: open   \u{00b7}   a: type an address   \u{00b7}   r: refresh",
            "",
            footer,
            p,
        )
    }

    fn peer_menu_view<'a>(
        &self,
        name: &str,
        peer: &resilience_core::share::Peer,
        actions: &[resilience_shell::app::PeerAction],
        p: &Palette,
    ) -> Element<'a, Message> {
        let rows = actions
            .iter()
            .map(|a| RowSpec {
                glyph: "\u{203a}",
                title: a.name().to_string(),
                ..RowSpec::default()
            })
            .collect();
        self.rows_view(
            rows,
            "Enter or click: open",
            "",
            format!(
                "{} at {}. Pages open in the text browser (q comes back). Anyone on this network can read them too.",
                resilience_core::printable(name),
                peer.display()
            ),
            p,
        )
    }

    fn choose_mode_view<'a>(
        &self,
        row: &resilience_core::admin::AccountRow,
        p: &Palette,
    ) -> Element<'a, Message> {
        let rows = resilience_shell::app::MODES
            .iter()
            .map(|m| RowSpec {
                glyph: "\u{25c9}",
                title: m.label().to_string(),
                sub: m.description().to_string(),
                right: if *m == row.effective {
                    "current".into()
                } else {
                    String::new()
                },
                mark: *m == row.effective,
                warn: false,
                header: false,
            })
            .collect();
        self.rows_view(
            rows,
            "Enter or click: continue   \u{00b7}   Esc: cancel",
            "",
            "Next you see what changes and confirm; nothing changes yet.".into(),
            p,
        )
    }

    fn packs_view<'a>(&self, s: &PacksScreen, p: &Palette) -> Element<'a, Message> {
        use resilience_core::human_bytes;
        let rows = s
            .rows()
            .into_iter()
            .map(|r| match r {
                PackRow::Header(c) => RowSpec {
                    title: c.label().to_uppercase(),
                    header: true,
                    ..RowSpec::default()
                },
                PackRow::OthersHeader => RowSpec {
                    title: "OTHER FILES (NOT IN THE CATALOG)".into(),
                    header: true,
                    ..RowSpec::default()
                },
                PackRow::Pack(i) => {
                    let pk = &s.cfg.packs[i];
                    let (mark, state) = s.row_state(r);
                    RowSpec {
                        glyph: tick_glyph(mark),
                        title: pk.title.clone(),
                        sub: format!(
                            "{}  ·  {}  ·  {}",
                            human_bytes(pk.size),
                            pk.language,
                            pk.description
                        ),
                        right: state,
                        mark: mark != "[ ]",
                        ..RowSpec::default()
                    }
                }
                PackRow::Other(j) => {
                    let o = &s.inv.others[j];
                    let (mark, state) = s.row_state(r);
                    RowSpec {
                        glyph: tick_glyph(mark),
                        title: o.file.clone(),
                        sub: format!(
                            "{}  ·  not in the catalog (an older version?), still readable",
                            human_bytes(o.size)
                        ),
                        right: state,
                        mark: mark != "[ ]",
                        ..RowSpec::default()
                    }
                }
            })
            .collect();
        let plan = s.plan();
        let mut footer = format!(
            "To add: {} ({})  ·  to remove: {}",
            plan.get.len(),
            human_bytes(plan.add_bytes),
            plan.remove.len()
        );
        if let Some(free) = s.free {
            footer.push_str(&format!("  ·  free space {}", human_bytes(free)));
            if plan.add_bytes > free + plan.remove_bytes {
                footer.push_str("  ·  NOT ENOUGH SPACE");
            }
        }
        footer.push_str(
            match &s.language {
                Some(l) => format!("  ·  language {l} (f: change)"),
                None => "  ·  all languages (f: choose one)".to_string(),
            }
            .as_str(),
        );
        self.rows_view(
            rows,
            "Click or Space: tick  ·  d download  ·  u from a USB stick  ·  s sets  ·  Enter details",
            "No resource packs are offered on this system.",
            footer,
            p,
        )
    }

    fn keys_view<'a>(
        &self,
        rows: &[resilience_shell::app::KeyRow],
        p: &Palette,
    ) -> Element<'a, Message> {
        use resilience_shell::app::KeyRow;
        let specs = rows
            .iter()
            .map(|r| {
                let line = self.app.key_row_text(r);
                // The text interface pads the name into a column: split there.
                let (title, sub) = match line.find("  ") {
                    Some(n) => (line[..n].trim().to_string(), line[n..].trim().to_string()),
                    None => (line.clone(), String::new()),
                };
                let own = match r {
                    KeyRow::Encryption | KeyRow::Signing | KeyRow::NoPgpKey => true,
                    KeyRow::Pgp(k) => k.secret,
                    KeyRow::Contact(_) => false,
                };
                RowSpec {
                    glyph: if own { "\u{2605}" } else { "\u{263a}" },
                    title,
                    warn: sub.contains("not created") || sub.contains("revoked"),
                    sub,
                    ..RowSpec::default()
                }
            })
            .collect();
        self.rows_view(
            specs,
            "Enter details  ·  a add a key file  ·  t type a key  ·  g OpenPGP  ·  e export  ·  Del remove",
            "",
            "Your keys come first, then your contacts. Check a contact's key with them in \
             person or by phone."
                .into(),
            p,
        )
    }

    fn choices_view<'a>(
        &self,
        items: &[resilience_shell::app::Choice],
        p: &Palette,
    ) -> Element<'a, Message> {
        let rows = items
            .iter()
            .enumerate()
            .map(|(i, c)| RowSpec {
                glyph: "\u{203a}",
                title: c.name().to_string(),
                sub: c.description().to_string(),
                right: format!("{}", i + 1),
                ..RowSpec::default()
            })
            .collect();
        self.rows_view(
            rows,
            "Enter, click or its number: open",
            "",
            String::new(),
            p,
        )
    }

    /// Dialogs drawn natively over the screen (the others use the text
    /// panel: ticking recipients, the clock, the installer's questions).
    fn native_modal(&self) -> bool {
        matches!(
            self.app.modal,
            Some(
                Modal::Confirm { .. }
                    | Modal::Message { .. }
                    | Modal::NewNote { .. }
                    | Modal::WifiPassword { .. }
                    | Modal::ManPage { .. }
                    | Modal::PackSearch { .. }
                    | Modal::ShareInput { .. }
                    | Modal::ChatInput { .. }
                    | Modal::ExpectedSum { .. }
            )
        )
    }

    fn dialog_view<'a>(&self, p: &Palette) -> Option<Element<'a, Message>> {
        use resilience_shell::app::{ChatInput, ShareInput};
        let p = *p;
        let m = self.app.modal.as_ref()?;
        let key = |c: KeyCode| Message::Press(KeyEvent::new(c, KeyModifiers::NONE));
        let cursor = if (self.app.tick / 2).is_multiple_of(2) {
            "\u{258f}"
        } else {
            " "
        };
        let btn = |label: String, msg: Message, primary: bool, danger: bool, focused: bool| {
            button(text(label).size(15).font(style::UI_FONT_BOLD))
                .on_press(msg)
                .padding([10, 20])
                .style(move |t, s| style::dialog_button(t, s, primary, danger, focused))
        };
        // (title, body, field shown, hint, error, buttons)
        let mut field: Option<String> = None;
        let mut hint = String::new();
        let mut err: Option<String> = None;
        let mut buttons: Vec<Element<'a, Message>> = Vec::new();
        let mut is_error = false;
        let ok_cancel = |ok: &str, buttons: &mut Vec<Element<'a, Message>>| {
            buttons.push(btn("Cancel".into(), key(KeyCode::Esc), false, false, false).into());
            buttons.push(btn(ok.into(), key(KeyCode::Enter), true, false, true).into());
        };
        let (title, body): (String, String) = match m {
            Modal::Message { title, body, error } => {
                is_error = *error;
                buttons.push(btn("OK".into(), key(KeyCode::Enter), true, *error, true).into());
                (sentence_case(title), body.clone())
            }
            Modal::Confirm {
                title,
                body,
                confirm,
                yes_focused,
                ..
            } => {
                buttons.push(
                    btn(
                        "Cancel".into(),
                        key(KeyCode::Char('n')),
                        false,
                        false,
                        !*yes_focused,
                    )
                    .into(),
                );
                buttons.push(
                    btn(
                        confirm.clone(),
                        key(KeyCode::Char('y')),
                        true,
                        false,
                        *yes_focused,
                    )
                    .into(),
                );
                hint = "y: confirm   \u{00b7}   n or Esc: cancel".into();
                (sentence_case(title), body.clone())
            }
            Modal::NewNote { value, error } => {
                field = Some(value.clone());
                err = error.clone();
                hint = "Letters, digits, spaces, - _ .".into();
                ok_cancel("Create", &mut buttons);
                ("New note".into(), "Name for the new note:".into())
            }
            Modal::WifiPassword {
                net, value, error, ..
            } => {
                field = Some("\u{2022}".repeat(value.chars().count()));
                err = error.clone();
                hint = "Usually on a label on the router.".into();
                ok_cancel("Connect", &mut buttons);
                (
                    "Wi-Fi password".into(),
                    format!(
                        "Password for the Wi-Fi network \u{201c}{}\u{201d}:",
                        net.ssid
                    ),
                )
            }
            Modal::ManPage { value, error } => {
                field = Some(value.clone());
                err = error.clone();
                hint =
                    "For example: ls, 5 passwd, copy files. In a page: q quits, / searches.".into();
                ok_cancel("Open", &mut buttons);
                (
                    "Manual pages".into(),
                    "Command name, or words to search the manuals for:".into(),
                )
            }
            Modal::PackSearch { item, value } => {
                field = Some(value.clone());
                hint = "Words to look for in the full text.".into();
                ok_cancel("Search", &mut buttons);
                (
                    "Search".into(),
                    format!("Search in \u{201c}{}\u{201d}:", item.title),
                )
            }
            Modal::ShareInput { kind, value, error } => {
                field = Some(value.clone());
                err = error.clone();
                let (t, ask, h, ok) = match kind {
                    ShareInput::Address => (
                        "Address of a computer",
                        "Address of the computer that shares (for example 192.168.1.20, or fe80::1%bat0 on the mesh):",
                        "A computer on a network this computer is on.",
                        "Open",
                    ),
                    ShareInput::Name => (
                        "Name others see",
                        "The name other computers will see (a place, not a person):",
                        "Up to 32 letters, digits, spaces or dashes.",
                        "Save",
                    ),
                };
                hint = h.into();
                ok_cancel(ok, &mut buttons);
                (t.into(), ask.into())
            }
            Modal::ChatInput { kind, value, error } => {
                field = Some(value.clone());
                err = error.clone();
                let (t, ask, h, ok) = match kind {
                    ChatInput::Address => (
                        "Write to an address",
                        "Address of the person (they find it under m in Local messages):"
                            .to_string(),
                        "32 characters, spaces allowed.",
                        "Continue",
                    ),
                    ChatInput::Text { peer } => (
                        "Write a message",
                        format!("Message for {}:", messages::short_address(peer)),
                        "Ctrl+N: new line. Kept and retried until delivered.",
                        "Send",
                    ),
                    ChatInput::FileText { peer, path, size } => (
                        "Send a file",
                        format!(
                            "File \u{201c}{}\u{201d} ({}) for {}. A message with it (optional):",
                            messages::clean_line(
                                &path
                                    .file_name()
                                    .map(|n| n.to_string_lossy())
                                    .unwrap_or_default(),
                                60
                            ),
                            messages::size_text(*size),
                            messages::short_address(peer)
                        ),
                        "Ctrl+N: new line. A copy is sent; it is retried until delivered.",
                        "Send",
                    ),
                    ChatInput::ContactName { .. } => (
                        "Name this person",
                        "Your own name for this address (empty: no name):".to_string(),
                        "Only you see it.",
                        "Save",
                    ),
                };
                hint = h.into();
                ok_cancel(ok, &mut buttons);
                (t.into(), ask)
            }
            Modal::ExpectedSum { file, value, error } => {
                field = Some(value.clone());
                err = error.clone();
                hint = "SHA-256, SHA-512, SHA-1 or MD5, as published.".into();
                ok_cancel("Compare", &mut buttons);
                (
                    "Compare with a checksum".into(),
                    format!(
                        "Checksum that \u{201c}{}\u{201d} should have:",
                        file.file_name()
                            .map(|n| n.to_string_lossy())
                            .unwrap_or_default()
                    ),
                )
            }
            _ => return None,
        };
        let mut c = column![
            text(title).size(22).font(style::UI_FONT_BOLD),
            text(body).size(15),
        ]
        .spacing(14);
        if let Some(v) = field {
            c = c.push(
                container(text(format!("{v}{cursor}")).size(17))
                    .padding([12, 16])
                    .width(Fill)
                    .style(move |_| style::field(&p)),
            );
        }
        if let Some(e) = err {
            c = c.push(text(e).size(14).color(p.error));
        } else if !hint.is_empty() {
            c = c.push(text(hint).size(13).style(style::dim_text));
        }
        c = c.push(row![Space::new().width(Fill), row(buttons).spacing(12)]);
        let card = container(c)
            .padding(28)
            .max_width(620.0)
            .style(move |_| style::dialog(&p, is_error));
        Some(iced::widget::opaque(
            container(card).center(Fill).padding(24).style(style::scrim),
        ))
    }

    fn wifi_view<'a>(
        &self,
        overview: Option<&'a resilience_core::wifi::Overview>,
        error: Option<&str>,
        p: &Palette,
    ) -> Element<'a, Message> {
        use resilience_core::wifi::Security;
        let p = *p;
        let hint = "Enter or click: join   \u{00b7}   d: disconnect   \u{00b7}   f: forget   \u{00b7}   r: scan again   \u{00b7}   w: switch Wi-Fi on";
        let problem = match (overview, error) {
            (_, Some(e)) => Some(format!(
                "Wi-Fi information could not be read: {e}. Press r to try again."
            )),
            (Some(o), None) if o.devices == 0 => Some(
                "No Wi-Fi adapter was found. If this computer has one, it may need firmware \
                 that is not included, or a hardware switch may be off. A network cable works \
                 without any setup."
                    .into(),
            ),
            (Some(o), None) if !o.radio_on => {
                Some("Wi-Fi is switched off. Press w to switch it on.".into())
            }
            (None, None) => Some("Looking for networks\u{2026}".into()),
            _ => None,
        };
        if let Some(msg) = problem {
            return column![
                centered_note(&msg),
                text(hint).size(13).style(style::dim_text)
            ]
            .padding([8, 28])
            .into();
        }
        let nets = overview.map_or(&[][..], |o| o.networks.as_slice());
        let sel = self.app.selected_row();
        let items: Vec<Element<'a, Message>> = nets
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let state = if n.in_use {
                    "Connected".to_string()
                } else if n.saved.is_some() {
                    "Saved".to_string()
                } else if let Some(pb) = n.problem() {
                    sentence_case(pb)
                } else if n.security == Security::Open {
                    "Open: anyone nearby can read unencrypted traffic".to_string()
                } else {
                    String::new()
                };
                button(
                    row![
                        signal_bars(n.signal, if n.in_use { p.accent } else { p.text }, p.dim),
                        column![
                            text(n.ssid.clone()).size(16).font(if n.in_use {
                                style::UI_FONT_BOLD
                            } else {
                                style::UI_FONT
                            }),
                            text(state)
                                .size(13)
                                .color(if n.in_use { p.accent } else { p.dim })
                        ]
                        .width(Fill),
                        text(n.security.label()).size(13).style(style::dim_text),
                        text(format!("{}%", n.signal))
                            .size(13)
                            .style(style::dim_text)
                            .width(Length::Fixed(48.0)),
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
        let list: Element<'a, Message> = if items.is_empty() {
            centered_note(
                "No networks found yet. Scanning takes a few seconds; press r to look again.",
            )
        } else {
            self.rows_shown.set(items.len());
            scrollable(column(items).spacing(4))
                .id(iced::widget::Id::new(LIST))
                .spacing(8)
                .height(Fill)
                .into()
        };
        column![
            row![Space::new().width(Fill), text(hint).size(13).style(style::dim_text)],
            list,
            text("Networks you join are saved for your account only. Other accounts on this computer do not see them.")
                .size(13)
                .style(style::dim_text),
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

/// One row of [`Gui::rows_view`].
#[derive(Default)]
struct RowSpec {
    glyph: &'static str,
    title: String,
    sub: String,
    right: String,
    /// The right-hand text in the accent colour.
    mark: bool,
    /// The second line in the warning colour.
    warn: bool,
    /// A section heading: not a button.
    header: bool,
}

/// Four rising bars, lit by signal strength (0-100).
fn signal_bars<'a>(signal: u8, on: iced::Color, off: iced::Color) -> Element<'a, Message> {
    let lit = usize::from(signal.min(100)).div_ceil(25).max(1);
    let mut r = row![].spacing(1).width(Length::Fixed(64.0));
    for (i, g) in ["\u{2582}", "\u{2584}", "\u{2586}", "\u{2588}"]
        .iter()
        .enumerate()
    {
        let c = if i < lit {
            on
        } else {
            iced::Color { a: 0.35, ..off }
        };
        r = r.push(text(*g).size(16).font(style::MONO_FONT).color(c));
    }
    r.into()
}

/// The first letter of a name, for its round mark in the lists.
/// The text interface's checkbox as one glyph.
fn tick_glyph(mark: &str) -> &'static str {
    match mark {
        "[+]" => "\u{2295}",
        "[-]" => "\u{2296}",
        "[*]" => "\u{2713}",
        _ => "\u{25cb}",
    }
}

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
