//! Rendering. Pure functions of [`App`] state (scroll offsets are clamped
//! here, which is the only mutation).
//!
//! Layout targets 80x24 as the minimum comfortable size and degrades to a
//! "terminal too small" notice below 40x12. Only box-drawing and block
//! characters present in the Linux console's default font are used.

use std::time::SystemTime;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Wrap};
use resilience_core::catalog::{Availability, Category};
use resilience_core::config::ThemeName;
use resilience_core::human_bytes;
use resilience_core::mode::Mode;
use resilience_core::network::{LinkLevel, ProbeOutcome};
use resilience_core::status::format_uptime;

use crate::app::{
    App, ChatInput, ChatRow, CommunityRow, Entry, InstallForm, InstallInput, InstallRow, Level,
    LibItem, LicencePage, MODES, MediaPurpose, Modal, NearbyRow, PackRow, PacksScreen, PeerAction,
    PeerPack, SearchHit, ShareForm, ShareInput, UpdateRow, View, category_description, level_tag,
};
use crate::help;
use crate::theme::Theme;
use resilience_core::admin::{AccountRow, Source};
use resilience_core::backup::{self, BackupFile};
use resilience_core::media::Volume;
use resilience_core::messages::{self, Direction, Name, Record, State, Status};
use resilience_core::packs::{PackKind, PackState};
use resilience_core::recovery;
use resilience_core::wifi::{self, Network, Overview};
use resilience_shell::list::ListState;

const LOGO: [&str; 2] = [
    "█▀█ █▀▀ █▀ █ █   █ █▀▀ █▄ █ █▀▀ █▀▀",
    "█▀▄ ██▄ ▄█ █ █▄▄ █ ██▄ █ ▀█ █▄▄ ██▄",
];
const SPINNER: [&str; 4] = ["|", "/", "-", "\\"];

pub use resilience_shell::fmt::{clock, date_time};

/// Renders `l` with the shell's selection, keeping ratatui's scroll offset.
fn render_list(f: &mut Frame, l: List, area: Rect, list: &mut resilience_shell::list::ListState) {
    let mut st = ratatui::widgets::ListState::default()
        .with_selected(list.selected())
        .with_offset(list.offset());
    f.render_stateful_widget(l, area, &mut st);
    // ratatui clamps a stale selection to the rows drawn (None when there
    // are none): keep that, as when the shell used ratatui's state.
    list.select(st.selected());
    list.set_offset(st.offset());
}

fn block<'a>(title: &'a str, theme: &Theme) -> Block<'a> {
    Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(theme.border)
        .title(Span::styled(format!(" {title} "), theme.title))
}

/// Status marker that carries meaning without colour.
fn marker(ok: Option<bool>, t: &Theme) -> Span<'static> {
    match ok {
        Some(true) => Span::styled("[OK]", t.ok),
        Some(false) => Span::styled("[!!]", t.warn),
        None => Span::styled("[--]", t.dim),
    }
}

/// The screen without the big header, for the GUI's text panel (ADR 0026):
/// the GUI draws its own title and status bars around it.
pub fn draw_panel(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let theme = Theme::new(app.theme);
    f.render_widget(Block::new().style(theme.base), area);
    let [body, footer] = Layout::vertical([Constraint::Min(0), Constraint::Length(2)]).areas(area);
    let body = match app.sharing_banner() {
        Some(b) => {
            let [line, rest] =
                Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(body);
            f.render_widget(Paragraph::new(Span::styled(b, theme.warn)), line);
            rest
        }
        None => body,
    };
    draw_body(f, app, body);
    draw_footer(f, app, footer);
    draw_modal(f, app, area);
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let theme = Theme::new(app.theme);
    f.render_widget(Block::new().style(theme.base), area);
    if area.width < 40 || area.height < 12 {
        let msg = Paragraph::new(format!(
            "Terminal too small ({}x{}).\nResilience needs at least 40x12; 80x24 recommended.",
            area.width, area.height
        ))
        .wrap(Wrap { trim: true })
        .style(theme.warn);
        f.render_widget(msg, area);
        return;
    }
    let tall = area.height >= 22 && area.width >= 60;
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(if tall { 5 } else { 3 }),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .areas(area);
    draw_header(f, app, header, tall);
    // While this computer shares (ADR 0025 §3), on every screen.
    let body = match app.sharing_banner() {
        Some(b) => {
            let [line, rest] =
                Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(body);
            f.render_widget(Paragraph::new(Span::styled(b, theme.warn)), line);
            rest
        }
        None => body,
    };
    draw_body(f, app, body);
    draw_footer(f, app, footer);
    draw_modal(f, app, area);
}

fn draw_header(f: &mut Frame, app: &App, area: Rect, tall: bool) {
    let t = &Theme::new(app.theme);
    let b = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(t.border);
    let inner = b.inner(area);
    f.render_widget(b, area);
    let right = vec![
        Line::from(vec![Span::styled(clock(SystemTime::now()), t.accent)]),
        Line::from(Span::styled(
            format!("{} v{}", app.sys.edition.to_uppercase(), app.sys.version),
            t.dim,
        )),
        Line::from(Span::styled(format!("MODE: {}", app.mode()), t.accent)),
    ];
    if tall {
        let lines = vec![
            Line::from(Span::styled(LOGO[0], t.title)),
            Line::from(Span::styled(LOGO[1], t.title)),
            Line::from(Span::styled("OS :: AUTONOMOUS INFORMATION SYSTEM", t.base)),
        ];
        f.render_widget(Paragraph::new(lines), inner);
        f.render_widget(Paragraph::new(right).alignment(Alignment::Right), inner);
    } else {
        let line = Line::from(vec![
            Span::styled("RESILIENCE OS", t.title),
            Span::styled(" :: AUTONOMOUS INFORMATION SYSTEM", t.base),
        ]);
        f.render_widget(Paragraph::new(line), inner);
        let compact = format!("{}  {}", app.mode(), clock(SystemTime::now()));
        f.render_widget(
            Paragraph::new(Span::styled(compact, t.accent)).alignment(Alignment::Right),
            inner,
        );
    }
}

fn footer_hints(view: &View) -> &'static str {
    match view {
        View::Home { .. } => "1-9/Enter Open  / Search  F1 Help  F5 Status  q Log out",
        View::Category { .. } => "Enter Open  Esc Back  / Search  F1 Help  F5 Status",
        View::Handbook { .. } => "Enter Read  Esc Back  F1 Help",
        View::Pager { .. } | View::Status { .. } | View::Help { .. } => {
            "Up/Down PgUp/PgDn Scroll  Esc Back"
        }
        View::Notes { .. } => "Enter Open  n New note  r Refresh  Esc Back",
        View::Chats { .. } => {
            "Enter Open  n Write to an address  p People nearby  m My address  Esc"
        }
        View::Nearby { .. } => "Enter Write  a Name this person  i Address  r Refresh  Esc Back",
        View::Chat { .. } => {
            "w Write  f File  o Open  s Save  c Cancel  a Name  i Address  Esc Back"
        }
        View::Network { .. } => "p Probe Internet  r Refresh  Esc Back",
        View::Community { .. } => "Enter Open  a Type an address  r Refresh  Esc Back  F1 Help",
        View::PeerMenu { .. } => "Enter Open  Esc Back  F1 Help",
        View::PeerPacks { .. } => "Space Tick  Enter/d Get the ticked packs  Esc Back",
        View::PeerFiles { .. } => "Enter Copy into Received  Esc Back  F1 Help",
        View::ShareSetup { .. } => "Enter/Space Change  Up/Down Move  Esc Cancel  F1 Help",
        View::Settings { .. } => "Enter Apply  Esc Back",
        View::Install { .. } => "Enter Change  m Mode  f Full name  a Add  d Remove  Esc  F1 Help",
        View::InstallDisks { .. } => "Enter Choose this disk  r Refresh  Esc Back",
        View::InstallLayouts { .. } => "Enter Choose  Esc Back",
        View::Search { .. } => "Type to search  Up/Down Select  Enter Open  Esc Back",
        View::Users { .. } => "Enter Change mode  r Refresh  Esc Back  F1 Help",
        View::ChooseMode { .. } => "Up/Down Choose  Enter Continue  Esc Cancel",
        View::Recovery { .. } => "Number/Enter Open  Esc Back  F1 Help",
        View::Generations { .. } => "Enter Make default  r Refresh  Esc Back  F1 Help",
        View::Updates { .. } => "Enter Open/Install  c Check  r Refresh  Esc Back  F1 Help",
        View::Licence { .. } => {
            "Tab Summary/Text  Up/Down Scroll  Space Tick  Enter Confirm  q Log out"
        }
        View::Media { .. } => "Enter Open  e Eject  r Refresh  Esc Back  F1 Help",
        View::Wifi { .. } => "Enter Join  d Disconnect  f Forget  r Scan  w Wi-Fi on  Esc Back",
        View::Backups { .. } => "Enter Back up/Restore  c Check  e Eject  Esc Back  F1 Help",
        View::Packs { .. } => "Space Tick  s Sets  d Download  u From USB  f Language  Enter Info",
        View::PackSets { .. } => "Enter Tick this set  Esc Back",
        View::Files { pick: Some(_), .. } => {
            "Type a name  Enter Choose this file  F3 Details  F2 Rescan  Esc Cancel"
        }
        View::Files { .. } => "Type a name  Enter View  F3 Details & checksum  F2 Rescan  Esc",
        View::Choices { .. } => "Enter Open  Esc Back  F1 Help",
        View::Keys { .. } => {
            "Enter Open  a Add file  t Type key  g OpenPGP key  e Export  Del Remove"
        }
        View::Table(_) => "Arrows Rows & columns  PgUp/PgDn  Esc Back",
        View::Library {
            kind: PackKind::Map,
            ..
        } => "Enter Open the map  c Check  u USB stick  p Get packs  Esc Back",
        View::Library { .. } => "Enter Read  s Search  c Check  u USB stick  p Get packs  Esc Back",
    }
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let t = &Theme::new(app.theme);
    let [keys, status] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);
    f.render_widget(
        Paragraph::new(Span::styled(footer_hints(app.view()), t.accent)),
        keys,
    );
    let line = match app.current_toast() {
        Some(n) => {
            let style = match n.level {
                Level::Info => t.base,
                Level::Warn => t.warn,
                Level::Error => t.error,
            };
            Line::from(vec![
                Span::styled(level_tag(n.level), style),
                Span::raw(" "),
                Span::styled(n.text.clone(), style),
            ])
        }
        None => Line::from(Span::styled("Ready.", t.dim)),
    };
    f.render_widget(Paragraph::new(line), status);
    if let Some(j) = &app.wifi_job {
        let s = format!(
            "connecting to '{}' {}s {}",
            j.ssid,
            j.started.elapsed().as_secs(),
            SPINNER[(app.tick as usize / 2) % SPINNER.len()]
        );
        f.render_widget(
            Paragraph::new(Span::styled(s, t.warn)).alignment(Alignment::Right),
            status,
        );
    }
    if app.probe.running.is_some() {
        let s = format!(
            "probing network {}",
            SPINNER[(app.tick as usize / 2) % SPINNER.len()]
        );
        f.render_widget(
            Paragraph::new(Span::styled(s, t.warn)).alignment(Alignment::Right),
            status,
        );
    }
}

fn draw_body(f: &mut Frame, app: &mut App, area: Rect) {
    // Take the top view out to satisfy the borrow checker; always put it back.
    let mut view = app.stack.pop().expect("view stack is never empty");
    match &mut view {
        View::Home { list } => draw_home(f, app, area, list),
        View::Category { cat, list } => draw_category(f, app, area, *cat, list),
        View::Handbook { pages, list, error } => {
            let t = Theme::new(app.theme);
            if let Some(e) = error {
                let p = Paragraph::new(format!("The handbook could not be opened.\n\n{e}"))
                    .wrap(Wrap { trim: false })
                    .style(t.error)
                    .block(block("RESILIENCE HANDBOOK", &t));
                f.render_widget(p, area);
            } else {
                let items: Vec<ListItem> = pages
                    .iter()
                    .map(|p| ListItem::new(p.title.clone()))
                    .collect();
                draw_list(
                    f,
                    &t,
                    area,
                    "RESILIENCE HANDBOOK",
                    items,
                    list,
                    "No handbook pages are installed.",
                );
            }
        }
        View::Pager {
            title,
            text,
            scroll,
        } => {
            let lines: Vec<Line> = text.lines().map(|l| Line::from(l.to_string())).collect();
            draw_scroll(f, &Theme::new(app.theme), area, title, lines, scroll);
        }
        View::Help { topic, scroll } => {
            let lines = help::lines(*topic, app.mode())
                .into_iter()
                .map(Line::from)
                .collect();
            draw_scroll(f, &Theme::new(app.theme), area, "HELP", lines, scroll);
        }
        View::Status { scroll } => {
            let lines = status_lines(app);
            draw_scroll(
                f,
                &Theme::new(app.theme),
                area,
                "SYSTEM STATUS",
                lines,
                scroll,
            );
        }
        View::Network { scroll } => {
            let lines = network_lines(app);
            draw_scroll(
                f,
                &Theme::new(app.theme),
                area,
                "NETWORK STATUS & DIAGNOSTICS",
                lines,
                scroll,
            );
        }
        View::Notes { notes, list } => {
            let t = Theme::new(app.theme);
            let items: Vec<ListItem> = notes
                .iter()
                .map(|n| ListItem::new(format!("{:<40} {:>10}", n.name, human_bytes(n.size))))
                .collect();
            draw_list(
                f,
                &t,
                area,
                "NOTES (~/Notes)",
                items,
                list,
                "No notes yet. Press 'n' to create one.",
            );
        }
        View::Chats { rows, status, list } => draw_chats(f, app, area, rows, status, list),
        View::Nearby { rows, list } => draw_nearby(f, app, area, rows, list),
        View::Community { rows, list } => draw_community(f, app, area, rows, list),
        View::PeerMenu {
            name,
            peer,
            actions,
            list,
        } => draw_peer_menu(f, app, area, name, peer, actions, list),
        View::PeerPacks {
            peer,
            rows,
            ticked,
            list,
        } => draw_peer_packs(f, app, area, peer, rows, ticked, list),
        View::PeerFiles { peer, rows, list } => draw_peer_files(f, app, area, peer, rows, list),
        View::ShareSetup { form, list } => draw_share_setup(f, app, area, form, list),
        View::Chat {
            peer,
            name,
            records,
            list,
        } => draw_chat(f, app, area, peer, name, records, list),
        View::Settings { list } => draw_settings(f, app, area, list),
        View::Install { form, list } => draw_install(f, app, area, form, list),
        View::InstallDisks { disks, error, list } => {
            draw_install_disks(f, app, area, disks, error.as_deref(), list)
        }
        View::InstallLayouts { list } => {
            let t = Theme::new(app.theme);
            let items: Vec<ListItem> = app
                .install_form()
                .map(|fm| fm.cfg.keymaps.clone())
                .unwrap_or_default()
                .iter()
                .map(|k| ListItem::new(app.sys.keyboard.label(k).to_string()))
                .collect();
            draw_list(
                f,
                &t,
                area,
                "KEYBOARD OF THE INSTALLED SYSTEM",
                items,
                list,
                "",
            );
        }
        View::Search { query, hits, list } => draw_search(f, app, area, query, hits, list),
        View::Users { rows, error, list } => draw_users(f, app, area, rows, error.as_deref(), list),
        View::ChooseMode { row, list } => draw_choose_mode(f, app, area, row, list),
        View::Recovery { list } => draw_recovery(f, app, area, list),
        View::Generations { gens, list } => draw_generations(f, app, area, gens, list),
        View::Licence {
            page,
            text,
            scroll,
            full_seen,
            ticked,
        } => draw_licence(f, app, area, *page, text, scroll, *full_seen, *ticked),
        View::Updates { rows, error, list } => {
            draw_updates(f, app, area, rows, error.as_deref(), list);
        }
        View::Wifi {
            overview,
            error,
            list,
        } => draw_wifi(f, app, area, overview.as_ref(), error.as_deref(), list),
        View::Media {
            volumes,
            error,
            list,
            purpose,
        } => draw_media(f, app, area, volumes, error.as_deref(), list, *purpose),
        View::PackSets { list } => draw_pack_sets(f, app, area, list),
        View::Files {
            idx,
            query,
            list,
            pick,
        } => draw_files(f, app, area, idx, query, list, *pick),
        View::Choices { title, items, list } => draw_choices(f, app, area, title, items, list),
        View::Keys { rows, list } => draw_keys(f, app, area, rows, list),
        View::Table(tbl) => draw_table(f, app, area, tbl),
        View::Library { kind, items, list } => draw_library(f, app, area, *kind, items, list),
        View::Packs { s, list } => draw_packs(f, app, area, s, list),
        View::Backups {
            volume,
            mount,
            files,
            list,
        } => draw_backups(f, app, area, volume, mount, files, list),
    }
    app.stack.push(view);
}

fn net_summary(app: &App) -> (Option<bool>, String) {
    if app.probe.running.is_some() {
        return (None, "PROBING...".into());
    }
    match (app.net.level, &app.probe.report) {
        (LinkLevel::Routed, Some(r)) if r.internet.is_ok() => {
            (Some(true), format!("ONLINE ({})", clock(r.finished)))
        }
        (LinkLevel::Routed, Some(_)) => (Some(false), "ROUTE UP, NO INTERNET".into()),
        (LinkLevel::Routed, None) => (None, "ROUTE UP (UNVERIFIED)".into()),
        (LinkLevel::LocalLink, _) => (None, "LOCAL LINK ONLY".into()),
        (LinkLevel::Offline, _) => (None, "OFFLINE".into()),
    }
}

fn status_panel(app: &App) -> Vec<Line<'static>> {
    let t = &Theme::new(app.theme);
    let s = &app.snapshot;
    let row = |label: &str, m: Span<'static>, value: String| {
        Line::from(vec![
            Span::styled(format!("{label:<8}"), t.dim),
            m,
            Span::raw(" "),
            Span::styled(value, t.base),
        ])
    };
    let failed = app.failed_units.len();
    let sys_value = if failed > 0 {
        format!("{} ({failed} failed)", app.sys_state.label())
    } else {
        app.sys_state.label()
    };
    let (net_ok, net_text) = net_summary(app);
    let power = match (s.batteries.first(), s.ac_power) {
        (Some(b), _) => {
            let low = b.capacity.is_some_and(|c| c <= 15) && b.status == "Discharging";
            let cap = b
                .capacity
                .map(|c| format!("{c}%"))
                .unwrap_or_else(|| "?%".into());
            (Some(!low), format!("BAT {cap} {}", b.status.to_uppercase()))
        }
        (None, Some(true)) => (Some(true), "AC POWER".into()),
        _ => (None, "NO BATTERY".into()),
    };
    let storage = match &s.home {
        Some(st) => {
            let low = st.total > 0 && st.available * 20 < st.total; // < 5 %
            (Some(!low), format!("{} FREE", human_bytes(st.available)))
        }
        None => (None, "UNKNOWN".into()),
    };
    let mem = match s.memory {
        Some(m) => {
            let used = m.total_kib.saturating_sub(m.available_kib) * 1024;
            (
                Some(m.available_kib * 10 >= m.total_kib),
                format!(
                    "{} / {}",
                    human_bytes(used),
                    human_bytes(m.total_kib * 1024)
                ),
            )
        }
        None => (None, "UNKNOWN".into()),
    };
    vec![
        row(
            "SYSTEM",
            marker(Some(app.sys_state.is_ok() && failed == 0), t),
            sys_value,
        ),
        row(
            "MODE",
            Span::styled("[--]", t.dim),
            app.mode().label().into(),
        ),
        row("NETWORK", marker(net_ok, t), net_text),
        row("POWER", marker(power.0, t), power.1),
        row("STORAGE", marker(storage.0, t), storage.1),
        row("MEMORY", marker(mem.0, t), mem.1),
        row(
            "UPTIME",
            Span::styled("[--]", t.dim),
            s.uptime.map(format_uptime).unwrap_or_else(|| "?".into()),
        ),
        row(
            "USER",
            Span::styled("[--]", t.dim),
            format!("{}@{}", app.who.user, s.hostname),
        ),
    ]
}

fn draw_home(f: &mut Frame, app: &App, area: Rect, list: &mut ListState) {
    let t = Theme::new(app.theme);
    let wide = area.width >= 76;
    let [status_area, menu_area] = if wide {
        Layout::horizontal([Constraint::Length(38), Constraint::Min(0)]).areas(area)
    } else {
        Layout::vertical([Constraint::Length(10), Constraint::Min(0)]).areas(area)
    };
    f.render_widget(
        Paragraph::new(status_panel(app)).block(block("STATUS", &t)),
        status_area,
    );

    let cats = app.home_categories();
    let items: Vec<ListItem> = cats
        .iter()
        .enumerate()
        .map(|(i, c)| ListItem::new(format!("[{}] {}", i + 1, c.title())))
        .collect();
    let b = block("MAIN MENU", &t);
    let inner = b.inner(menu_area);
    f.render_widget(b, menu_area);
    let desc_h = if inner.height > cats.len() as u16 + 3 {
        2
    } else {
        0
    };
    let [list_area, desc_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(desc_h)]).areas(inner);
    let l = List::new(items)
        .style(t.base)
        .highlight_style(t.selected)
        .highlight_symbol("> ");
    render_list(f, l, list_area, list);
    if let Some(c) = list.selected().and_then(|i| cats.get(i)) {
        f.render_widget(
            Paragraph::new(category_description(*c))
                .style(t.dim)
                .wrap(Wrap { trim: true }),
            desc_area,
        );
    }
}

fn entry_detail(app: &App, e: Entry) -> Vec<Line<'static>> {
    let t = &Theme::new(app.theme);
    let mode = app.mode();
    match e {
        Entry::Builtin(b) => vec![
            Line::from(Span::styled(b.name().to_string(), t.accent)),
            Line::from(""),
            Line::from(b.description().to_string()),
            Line::from(""),
            Line::from(Span::styled("Built into the Resilience interface.", t.dim)),
        ],
        Entry::App(i) => {
            let a = &app.catalog.apps[i];
            let (m, state) = match a.availability(mode) {
                Availability::Ready(_) => {
                    (marker(Some(true), t), "Installed and ready".to_string())
                }
                Availability::Missing(p) => (marker(Some(false), t), format!("Unavailable: {p}")),
            };
            let mut lines = vec![
                Line::from(Span::styled(a.name.clone(), t.accent)),
                Line::from(""),
                Line::from(a.description.clone()),
                Line::from(""),
                Line::from(vec![m, Span::raw(" "), Span::raw(state)]),
            ];
            if mode == Mode::UltraStable && a.restricted_command.is_some() {
                lines.push(Line::from(Span::styled(
                    "Runs in restricted mode for your account.",
                    t.dim,
                )));
            }
            lines
        }
    }
}

fn draw_category(f: &mut Frame, app: &App, area: Rect, cat: Category, list: &mut ListState) {
    let t = Theme::new(app.theme);
    let entries = app.entries(cat);
    let items: Vec<ListItem> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let num = if i < 9 {
                format!("[{}] ", i + 1)
            } else {
                "    ".into()
            };
            let suffix = if app.entry_ready(*e) {
                ""
            } else {
                "  [UNAVAILABLE]"
            };
            ListItem::new(format!("{num}{}{suffix}", app.entry_name(*e)))
        })
        .collect();
    let [list_area, detail_area] = if area.width >= 76 {
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(area)
    } else {
        Layout::vertical([Constraint::Min(0), Constraint::Length(7)]).areas(area)
    };
    draw_list(
        f,
        &t,
        list_area,
        cat.title(),
        items,
        list,
        "Nothing is available here for your account.",
    );
    if let Some(e) = list.selected().and_then(|i| entries.get(i)) {
        let p = Paragraph::new(entry_detail(app, *e))
            .style(t.base)
            .wrap(Wrap { trim: true })
            .block(block("DETAILS", &t));
        f.render_widget(p, detail_area);
    }
}

fn draw_list(
    f: &mut Frame,
    t: &Theme,
    area: Rect,
    title: &str,
    items: Vec<ListItem>,
    list: &mut ListState,
    empty: &str,
) {
    let b = block(title, t);
    if items.is_empty() {
        f.render_widget(
            Paragraph::new(empty.to_string())
                .style(t.dim)
                .wrap(Wrap { trim: true })
                .block(b),
            area,
        );
        return;
    }
    let l = List::new(items)
        .style(t.base)
        .highlight_style(t.selected)
        .highlight_symbol("> ")
        .block(b);
    render_list(f, l, area, list);
}

/// Word-wrap plain lines to `width` columns (by character count).
pub fn wrap(lines: Vec<Line<'static>>, width: usize) -> Vec<Line<'static>> {
    let width = width.max(10);
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        if line.width() <= width {
            out.push(line);
            continue;
        }
        // Styled lines that overflow are flattened to the first span's style.
        let style = line.spans.first().map(|s| s.style).unwrap_or_default();
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        let indent: String = text.chars().take_while(|c| *c == ' ').collect();
        let mut cur = String::new();
        for word in text.split(' ').filter(|w| !w.is_empty()) {
            let need = cur.chars().count() + usize::from(!cur.is_empty()) + word.chars().count();
            if !cur.is_empty() && need > width {
                out.push(Line::styled(std::mem::take(&mut cur), style));
                cur.push_str(&indent);
            }
            if !cur.is_empty() && !cur.ends_with(' ') {
                cur.push(' ');
            }
            // Hard-split words longer than the width.
            let mut w = word.to_string();
            while w.chars().count() > width {
                let head: String = w.chars().take(width).collect();
                w = w.chars().skip(width).collect();
                if !cur.trim().is_empty() {
                    out.push(Line::styled(std::mem::take(&mut cur), style));
                }
                out.push(Line::styled(head, style));
            }
            cur.push_str(&w);
        }
        if !cur.is_empty() {
            out.push(Line::styled(cur, style));
        }
    }
    out
}

fn draw_scroll(
    f: &mut Frame,
    t: &Theme,
    area: Rect,
    title: &str,
    lines: Vec<Line<'static>>,
    scroll: &mut usize,
) {
    let inner_w = area.width.saturating_sub(2) as usize;
    let inner_h = area.height.saturating_sub(2) as usize;
    let lines = wrap(lines, inner_w);
    let max = lines.len().saturating_sub(inner_h);
    *scroll = (*scroll).min(max);
    let pos = if max == 0 {
        String::new()
    } else {
        format!(" {}/{} ", *scroll + 1, max + 1)
    };
    let b = block(title, t).title_bottom(Line::from(Span::styled(pos, t.dim)).right_aligned());
    let visible: Vec<Line> = lines.into_iter().skip(*scroll).take(inner_h).collect();
    f.render_widget(
        Paragraph::new(Text::from(visible)).style(t.base).block(b),
        area,
    );
}

fn kv(t: &Theme, k: &str, v: impl Into<String>) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {k:<20}"), t.dim),
        Span::styled(v.into(), t.base),
    ])
}

fn heading(t: &Theme, s: &str) -> Line<'static> {
    Line::from(Span::styled(format!("== {s} =="), t.accent))
}

/// The lines of a scroll page as plain text (the GUI draws them itself).
pub fn plain(lines: Vec<Line<'static>>) -> String {
    lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn status_text(app: &App) -> String {
    plain(status_lines(app))
}

pub fn network_text(app: &App) -> String {
    plain(network_lines(app))
}

fn status_lines(app: &App) -> Vec<Line<'static>> {
    let t = &Theme::new(app.theme);
    let s = &app.snapshot;
    let mut l = vec![heading(t, "SYSTEM")];
    let state_ok = app.sys_state.is_ok() && app.failed_units.is_empty();
    l.push(Line::from(vec![
        Span::raw("  "),
        marker(Some(state_ok), t),
        Span::raw(format!(" systemd reports: {}", app.sys_state.label())),
    ]));
    if app.failed_units.is_empty() {
        l.push(kv(t, "Failed services", "none"));
    } else {
        l.push(kv(t, "Failed services", app.failed_units.len().to_string()));
        for u in &app.failed_units {
            l.push(Line::from(Span::styled(format!("    [XX] {u}"), t.error)));
        }
    }
    l.push(kv(t, "Operating system", s.os_name.clone()));
    l.push(kv(
        t,
        "Edition",
        format!(
            "{} (userland v{})",
            app.sys.edition,
            resilience_core::VERSION
        ),
    ));
    l.push(kv(t, "Kernel", s.kernel.clone()));
    l.push(kv(t, "Hostname", s.hostname.clone()));
    l.push(kv(
        t,
        "Uptime",
        s.uptime
            .map(format_uptime)
            .unwrap_or_else(|| "unknown".into()),
    ));
    if let Some([a, b, c]) = s.load {
        l.push(kv(t, "Load average", format!("{a:.2} {b:.2} {c:.2}")));
    }
    l.push(kv(
        t,
        "System generation",
        s.system_generation
            .clone()
            .unwrap_or_else(|| "not a NixOS system".into()),
    ));
    l.push(Line::from(""));
    l.push(heading(t, "ACCOUNT & SECURITY MODE"));
    l.push(kv(t, "User", app.who.user.clone()));
    l.push(kv(t, "Mode", app.mode().label()));
    l.push(kv(
        t,
        "Groups",
        if app.who.groups.is_empty() {
            "(none)".into()
        } else {
            app.who.groups.join(", ")
        },
    ));
    l.push(Line::from(format!("  {}", app.mode().description())));
    l.push(Line::from(""));
    l.push(heading(t, "STORAGE & MEMORY"));
    for (name, st) in [("System (/)", &s.root), ("Your files (home)", &s.home)] {
        match st {
            Some(st) => l.push(kv(
                t,
                name,
                format!(
                    "{} free of {}",
                    human_bytes(st.available),
                    human_bytes(st.total)
                ),
            )),
            None => l.push(kv(t, name, "unknown")),
        }
    }
    if let Some(m) = s.memory {
        l.push(kv(
            t,
            "Memory available",
            format!(
                "{} of {}",
                human_bytes(m.available_kib * 1024),
                human_bytes(m.total_kib * 1024)
            ),
        ));
    }
    l.push(Line::from(""));
    l.push(heading(t, "POWER"));
    if s.batteries.is_empty() {
        l.push(kv(t, "Battery", "none detected"));
    }
    for b in &s.batteries {
        let cap = b
            .capacity
            .map(|c| format!("{c}%"))
            .unwrap_or_else(|| "unknown".into());
        l.push(kv(
            t,
            &format!("Battery {}", b.name),
            format!("{cap}, {}", b.status),
        ));
    }
    l.push(kv(
        t,
        "Mains power",
        match s.ac_power {
            Some(true) => "connected",
            Some(false) => "disconnected",
            None => "no adapter reported",
        },
    ));
    l.push(Line::from(""));
    l.push(heading(t, "APPLICATIONS"));
    let mode = app.mode();
    let visible: Vec<_> = app
        .catalog
        .apps
        .iter()
        .filter(|a| mode.allows(a.min_mode))
        .collect();
    let missing: Vec<_> = visible
        .iter()
        .filter(|a| !matches!(a.availability(mode), Availability::Ready(_)))
        .collect();
    l.push(kv(
        t,
        "Available to you",
        format!("{} programs, {} unavailable", visible.len(), missing.len()),
    ));
    for a in missing {
        l.push(Line::from(Span::styled(
            format!("    [!!] {} ({})", a.name, a.id),
            t.warn,
        )));
    }
    l
}

fn outcome_line(t: &Theme, label: &str, o: &ProbeOutcome) -> Line<'static> {
    let (m, text) = match o {
        ProbeOutcome::Ok { detail, elapsed } => (
            marker(Some(true), t),
            format!("{detail} in {} ms", elapsed.as_millis()),
        ),
        ProbeOutcome::Failed(e) => (marker(Some(false), t), e.clone()),
        ProbeOutcome::Skipped(why) => (marker(None, t), why.clone()),
    };
    Line::from(vec![
        Span::styled(format!("  {label:<20}"), t.dim),
        m,
        Span::raw(format!(" {text}")),
    ])
}

fn network_lines(app: &App) -> Vec<Line<'static>> {
    let t = &Theme::new(app.theme);
    let n = &app.net;
    let (ok, summary) = net_summary(app);
    let mut l = vec![
        heading(t, "SUMMARY"),
        Line::from(vec![
            Span::raw("  "),
            marker(ok, t),
            Span::styled(format!(" {summary}"), t.accent),
        ]),
        Line::from(match n.level {
            LinkLevel::Offline => {
                "  No network cable or wireless connection is active. Everything stored on this computer keeps working."
            }
            LinkLevel::LocalLink => {
                "  Connected to a local network, but there is no route to other networks. Local services may work."
            }
            LinkLevel::Routed => {
                "  A route to other networks exists. Press 'p' to test whether the Internet is actually reachable."
            }
        }),
        Line::from(""),
        heading(t, "INTERFACES"),
    ];
    let mut any = false;
    for i in n
        .interfaces
        .iter()
        .filter(|i| i.kind != resilience_core::network::IfKind::Loopback)
    {
        any = true;
        let addrs = if i.addresses.is_empty() {
            "no address".to_string()
        } else {
            i.addresses
                .iter()
                .map(|a| a.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let state = if i.is_connected() {
            "connected"
        } else if i.operstate == "up" {
            "up, no carrier"
        } else {
            "disconnected"
        };
        l.push(Line::from(vec![
            Span::raw("  "),
            marker(Some(i.is_connected()), t),
            Span::styled(
                format!(" {:<10} {:<9} {state}", i.name, i.kind.label()),
                t.base,
            ),
        ]));
        l.push(Line::from(Span::styled(format!("       {addrs}"), t.dim)));
    }
    if !any {
        l.push(Line::from("  No network hardware detected."));
    }
    l.push(Line::from(""));
    l.push(heading(t, "ROUTING & DNS"));
    l.push(kv(
        t,
        "IPv4 gateway",
        match &n.gateway_v4 {
            Some((ifc, gw)) => {
                let seen = match n.gateway_arp {
                    Some(true) => "answered recently (ARP)",
                    _ => "not seen on the wire yet",
                };
                format!("{gw} via {ifc}, {seen}")
            }
            None => "none".into(),
        },
    ));
    l.push(kv(
        t,
        "IPv6 default route",
        n.default_v6_iface
            .clone()
            .map(|i| format!("via {i}"))
            .unwrap_or_else(|| "none".into()),
    ));
    l.push(kv(
        t,
        "DNS servers",
        if n.nameservers.is_empty() {
            "none configured".into()
        } else {
            n.nameservers
                .iter()
                .map(|a| a.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        },
    ));
    l.push(Line::from(""));
    l.push(heading(t, "INTERNET PROBE (ON REQUEST ONLY)"));
    match (&app.probe.running, &app.probe.report) {
        (Some((_, started)), _) => l.push(Line::from(format!(
            "  Probing... ({} s)",
            started.elapsed().as_secs()
        ))),
        (None, Some(r)) => {
            l.push(Line::from(Span::styled(
                format!("  Last probe at {}", clock(r.finished)),
                t.dim,
            )));
            l.push(outcome_line(t, "Internet (TCP)", &r.internet));
            l.push(outcome_line(t, "DNS lookup", &r.dns));
        }
        (None, None) => l.push(Line::from(
            "  Not run. Resilience never contacts the Internet unless you press 'p'.",
        )),
    }
    let targets: Vec<String> = app
        .probe
        .config
        .targets
        .iter()
        .map(|a| a.to_string())
        .collect();
    l.push(Line::from(Span::styled(
        format!(
            "  The probe contacts {} and resolves '{}', each with a {} ms timeout.",
            if targets.is_empty() {
                "nothing".into()
            } else {
                targets.join(", ")
            },
            app.probe.config.dns_name,
            app.probe.config.timeout.as_millis()
        ),
        t.dim,
    )));
    l
}

fn draw_settings(f: &mut Frame, app: &App, area: Rect, list: &mut ListState) {
    let t = Theme::new(app.theme);
    let kb = &app.sys.keyboard;
    let rows = (ThemeName::ALL.len() + kb.layouts.len()) as u16;
    let [themes, info] =
        Layout::vertical([Constraint::Length(rows + 2), Constraint::Min(0)]).areas(area);
    let current = |yes: bool| if yes { "  (current)" } else { "" };
    let layout_now = kb.effective(app.settings.keyboard.as_deref());
    let items: Vec<ListItem> = ThemeName::ALL
        .iter()
        .map(|th| {
            ListItem::new(format!(
                "Theme     {}{}",
                th.label(),
                current(*th == app.settings.theme)
            ))
        })
        .chain(kb.layouts.iter().map(|l| {
            ListItem::new(format!(
                "Keyboard  {}{}",
                l.label,
                current(l.id == layout_now)
            ))
        }))
        .collect();
    draw_list(f, &t, themes, "THEME & KEYBOARD", items, list, "");
    let lines = vec![
        Line::from(vec![
            Span::styled("Security mode: ", t.dim),
            Span::styled(app.mode().label(), t.accent),
        ]),
        Line::from(app.mode().description()),
        Line::from(""),
        Line::from(Span::styled(
            "Your security mode comes from your account, not from a setting, so it cannot be changed \
             here. An administrator can change it; see the handbook page 'Security modes'.",
            t.dim,
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("Settings file: {}", app.settings_path.display()),
            t.dim,
        )),
    ];
    f.render_widget(
        Paragraph::new(lines)
            .style(t.base)
            .wrap(Wrap { trim: true })
            .block(block("ACCOUNT", &t)),
        info,
    );
}

// ----- installer (ADR 0022) -------------------------------------------------

fn draw_install(f: &mut Frame, app: &App, area: Rect, form: &InstallForm, list: &mut ListState) {
    let t = Theme::new(app.theme);
    let items: Vec<ListItem> = form
        .rows()
        .into_iter()
        .map(|r| {
            ListItem::new(match r {
                InstallRow::Disk => match &form.disk {
                    Some(d) => format!("Disk        {}  {}", d.path, d.describe()),
                    None => "Disk        (choose the disk to install on)".to_string(),
                },
                InstallRow::Encrypt => format!(
                    "Encryption  {}",
                    if form.encrypt {
                        "ON: a passphrase is asked at every start"
                    } else {
                        "off: anyone with the disk can read it"
                    }
                ),
                InstallRow::Keyboard => {
                    format!("Keyboard    {}", app.sys.keyboard.label(&form.keymap))
                }
                InstallRow::Account(i) => {
                    let a = &form.accounts[i];
                    format!(
                        "Account     {:<16} {:<13} {}",
                        a.name,
                        a.mode.label(),
                        a.description
                    )
                }
                InstallRow::AddAccount => "            + Add an account".to_string(),
                InstallRow::Install => "Install...".to_string(),
            })
        })
        .collect();
    let rows = items.len() as u16;
    let [top, info] =
        Layout::vertical([Constraint::Length(rows + 2), Constraint::Min(0)]).areas(area);
    draw_list(
        f,
        &t,
        top,
        "INSTALL RESILIENCE ON THIS COMPUTER",
        items,
        list,
        "",
    );
    let mut lines = vec![
        Line::from(format!(
            "Resilience {} will be installed for {}. The whole disk is erased; nothing \
             happens before the summary and your confirmation.",
            form.cfg.edition,
            form.firmware.label()
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Accounts: an everyday account and an administrator who only authorises \
             changes are proposed. Ultra-Stable: the safest, for children or kiosks; \
             Stable: everyday use; Advanced: administrator (sudo). Passwords are asked \
             during the installation.",
            t.dim,
        )),
    ];
    if !resilience_core::install::has_administrator(&form.accounts) {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "[!!] No Advanced account: nobody could change modes, install packs or update.",
            t.warn,
        )));
    }
    f.render_widget(
        Paragraph::new(lines)
            .style(t.base)
            .wrap(Wrap { trim: true })
            .block(block("ABOUT", &t)),
        info,
    );
}

fn draw_install_disks(
    f: &mut Frame,
    app: &App,
    area: Rect,
    disks: &[resilience_core::install::Disk],
    error: Option<&str>,
    list: &mut ListState,
) {
    let t = Theme::new(app.theme);
    let items: Vec<ListItem> = disks
        .iter()
        .map(|d| {
            let line = format!("{:<14} {}", d.path, d.describe());
            match &d.problem {
                None => ListItem::new(line),
                Some(why) => ListItem::new(format!("{line}  [--] {why}")).style(t.dim),
            }
        })
        .collect();
    let [top, info] = Layout::vertical([Constraint::Min(5), Constraint::Length(9)]).areas(area);
    let empty = error.unwrap_or("No disk found.");
    draw_list(f, &t, top, "CHOOSE THE DISK", items, list, empty);
    let mut lines = Vec::new();
    if let Some(d) = list.selected().and_then(|i| disks.get(i)) {
        lines.push(Line::from(format!("{}  {}", d.path, d.describe())));
        if d.parts.is_empty() {
            lines.push(Line::from(Span::styled(
                "Empty: no partitions found.",
                t.dim,
            )));
        }
        for p in d.parts.iter().take(5) {
            lines.push(Line::from(format!(
                "  {} {} {} {}",
                p.path,
                resilience_core::install::human_size(p.size),
                p.fstype,
                p.label
            )));
        }
        if d.parts.len() > 5 {
            lines.push(Line::from(format!("  ... and {} more", d.parts.len() - 5)));
        }
    }
    f.render_widget(
        Paragraph::new(lines)
            .style(t.base)
            .wrap(Wrap { trim: true })
            .block(block("ON THIS DISK NOW (ALL ERASED)", &t)),
        info,
    );
}

fn two_panes(area: Rect) -> [Rect; 2] {
    if area.width >= 76 {
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(area)
    } else {
        Layout::vertical([Constraint::Min(0), Constraint::Length(8)]).areas(area)
    }
}

fn details(f: &mut Frame, t: &Theme, area: Rect, lines: Vec<Line<'static>>) {
    f.render_widget(
        Paragraph::new(lines)
            .style(t.base)
            .wrap(Wrap { trim: true })
            .block(block("DETAILS", t)),
        area,
    );
}

fn draw_users(
    f: &mut Frame,
    app: &App,
    area: Rect,
    rows: &[AccountRow],
    error: Option<&str>,
    list: &mut ListState,
) {
    let t = Theme::new(app.theme);
    if let Some(e) = error {
        let p = Paragraph::new(format!(
            "The list of accounts could not be read.\n\n{e}\n\n\
             Security modes can still be changed by an administrator in the system \
             configuration (resilience.users.<name>.mode)."
        ))
        .wrap(Wrap { trim: false })
        .style(t.error)
        .block(block("USERS & SECURITY MODES", &t));
        f.render_widget(p, area);
        return;
    }
    let items: Vec<ListItem> = rows
        .iter()
        .map(|r| {
            let you = if r.name == app.who.user { " (you)" } else { "" };
            ListItem::new(format!("{:<18} {}{you}", r.name, r.effective.label()))
        })
        .collect();
    let [l, d] = two_panes(area);
    draw_list(
        f,
        &t,
        l,
        "USERS & SECURITY MODES",
        items,
        list,
        "No accounts are managed by Resilience on this system.",
    );
    if let Some(r) = list.selected().and_then(|i| rows.get(i)) {
        let mut lines = vec![
            kv(&t, "Account", r.name.clone()),
            kv(&t, "Security mode", r.effective.label()),
        ];
        if !r.description.is_empty() {
            lines.insert(1, kv(&t, "Name", r.description.clone()));
        }
        lines.push(kv(
            &t,
            "Set by",
            match r.source {
                Source::Declared => "system config".to_string(),
                Source::Override => "mode change".to_string(),
            },
        ));
        if r.active != r.effective {
            lines.push(Line::from(Span::styled(
                format!(
                    "[!!] Active permissions are still {}; restart to apply.",
                    r.active
                ),
                t.warn,
            )));
        }
        if r.autologin {
            lines.push(Line::from(Span::styled(
                "Logs in automatically on the first console, so it always stays Ultra-Stable.",
                t.dim,
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(r.effective.description()));
        details(f, &t, d, lines);
    }
}

fn draw_choose_mode(f: &mut Frame, app: &App, area: Rect, row: &AccountRow, list: &mut ListState) {
    let t = Theme::new(app.theme);
    let items: Vec<ListItem> = MODES
        .iter()
        .map(|m| {
            let cur = if *m == row.effective {
                "  (current)"
            } else {
                ""
            };
            ListItem::new(format!("{}{cur}", m.label()))
        })
        .collect();
    let [l, d] = two_panes(area);
    draw_list(
        f,
        &t,
        l,
        &format!("NEW MODE FOR '{}'", row.name),
        items,
        list,
        "",
    );
    if let Some(m) = list.selected().and_then(|i| MODES.get(i)) {
        let lines = vec![
            Line::from(Span::styled(m.label(), t.accent)),
            Line::from(""),
            Line::from(m.description()),
            Line::from(""),
            Line::from(Span::styled(
                "Enter shows the consequences and asks for confirmation. Nothing changes yet.",
                t.dim,
            )),
        ];
        details(f, &t, d, lines);
    }
}

fn draw_choices(
    f: &mut Frame,
    app: &App,
    area: Rect,
    title: &str,
    items: &[crate::app::Choice],
    list: &mut ListState,
) {
    let t = Theme::new(app.theme);
    let rows: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, c)| ListItem::new(format!("[{}] {}", i + 1, c.name())))
        .collect();
    let [l, d] = two_panes(area);
    draw_list(f, &t, l, title, rows, list, "");
    if let Some(c) = list.selected().and_then(|i| items.get(i)) {
        details(f, &t, d, vec![Line::from(c.description())]);
    }
}

fn draw_keys(
    f: &mut Frame,
    app: &App,
    area: Rect,
    rows: &[crate::app::KeyRow],
    list: &mut ListState,
) {
    let t = Theme::new(app.theme);
    let items: Vec<ListItem> = rows
        .iter()
        .map(|r| ListItem::new(app.key_row_text(r)))
        .collect();
    let contacts = rows
        .iter()
        .filter(|r| match r {
            crate::app::KeyRow::Contact(_) => true,
            crate::app::KeyRow::Pgp(k) => !k.secret,
            _ => false,
        })
        .count();
    let [l, d] = Layout::vertical([Constraint::Min(6), Constraint::Length(6)]).areas(area);
    draw_list(
        f,
        &t,
        l,
        &format!(
            "KEYS & CONTACTS ({contacts} contact{})",
            if contacts == 1 { "" } else { "s" }
        ),
        items,
        list,
        "",
    );
    let help = if contacts == 0 {
        "No contacts yet. To exchange encrypted files or check signatures, get the public \
         key file of the other person (a USB stick) and press a, or type their key with t. \
         Check the keys with them in person or by phone."
    } else {
        "Your keys come first, then your contacts. Enter shows the details. Encrypting for \
         a contact uses their encryption key; their signing key checks their signatures."
    };
    f.render_widget(
        Paragraph::new(help)
            .wrap(Wrap { trim: true })
            .style(t.dim)
            .block(block("ABOUT KEYS", &t)),
        d,
    );
}

fn draw_recovery(f: &mut Frame, app: &App, area: Rect, list: &mut ListState) {
    let t = Theme::new(app.theme);
    let all = app.recovery_items();
    let items: Vec<ListItem> = all
        .iter()
        .enumerate()
        .map(|(i, r)| ListItem::new(format!("[{}] {}", i + 1, r.name())))
        .collect();
    let [l, d] = two_panes(area);
    draw_list(f, &t, l, "RECOVERY & DIAGNOSTICS", items, list, "");
    if let Some(r) = list.selected().and_then(|i| all.get(i)) {
        details(f, &t, d, vec![Line::from(r.description())]);
    }
}

/// Short state of a generation for the list: "started, default".
fn generation_marks(g: &recovery::Generation) -> String {
    let mut marks = Vec::new();
    if g.booted {
        marks.push("started");
    } else if g.running {
        marks.push("active");
    }
    if g.default {
        marks.push("default");
    }
    marks.join(", ")
}

#[allow(clippy::too_many_arguments)]
fn draw_licence(
    f: &mut Frame,
    app: &App,
    area: Rect,
    page: LicencePage,
    text: &str,
    scroll: &mut usize,
    full_seen: bool,
    ticked: bool,
) {
    let t = Theme::new(app.theme);
    let title = app
        .sys
        .licence
        .as_ref()
        .map(|l| l.title())
        .unwrap_or_else(|| "licence".into());
    let [body, bottom] = Layout::vertical([Constraint::Min(3), Constraint::Length(4)]).areas(area);
    let (heading, content) = match page {
        LicencePage::Summary => (
            format!("{}: SUMMARY (1/2)", title.to_uppercase()),
            resilience_core::licence::SUMMARY,
        ),
        LicencePage::Full => (
            format!("{}: ORIGINAL TEXT (2/2)", title.to_uppercase()),
            text,
        ),
    };
    let lines: Vec<Line> = content.lines().map(|l| Line::from(l.to_string())).collect();
    draw_scroll(f, &t, body, &heading, lines, scroll);
    let mark = if ticked { "[x]" } else { "[ ]" };
    let hint = if !full_seen {
        "Press Tab to read the original text, then Space to tick and Enter to confirm."
    } else if !ticked {
        "Space ticks the box; Enter confirms; q logs out."
    } else {
        "Enter confirms; the date and time are recorded on this computer."
    };
    let style = if full_seen { t.base } else { t.dim };
    let p = Paragraph::new(vec![
        Line::styled(
            format!("{mark} I certify that I have read the {title}"),
            style,
        ),
        Line::styled(hint, t.dim),
    ])
    .wrap(Wrap { trim: false })
    .block(block("BEFORE YOU START", &t));
    f.render_widget(p, bottom);
}

fn draw_updates(
    f: &mut Frame,
    app: &App,
    area: Rect,
    rows: &[UpdateRow],
    error: Option<&str>,
    list: &mut ListState,
) {
    let t = Theme::new(app.theme);
    let title = "UPDATE FROM A USB STICK";
    if let Some(e) = error {
        let p = Paragraph::new(format!(
            "The list of USB sticks could not be read.\n\n{e}\n\nPress r to try again."
        ))
        .wrap(Wrap { trim: false })
        .style(t.error)
        .block(block(title, &t));
        f.render_widget(p, area);
        return;
    }
    let items: Vec<ListItem> = rows
        .iter()
        .map(|r| {
            let state = match (&r.volume.mountpoint, &r.claim) {
                _ if r.volume.problem.is_some() => "unusable".to_string(),
                (None, _) => "not open".to_string(),
                (Some(_), Some(Ok(p))) => format!("update {}", p.version),
                (Some(_), Some(Err(_))) => "unreadable update".to_string(),
                (Some(_), None) => "no update".to_string(),
            };
            ListItem::new(format!(
                "{:<14.14} {:>9} {state}",
                r.volume.display_name(),
                human_bytes(r.volume.size)
            ))
        })
        .collect();
    let [l, d] = two_panes(area);
    draw_list(
        f,
        &t,
        l,
        title,
        items,
        list,
        "No USB stick or memory card found. Plug in the stick with the update, wait a \
         few seconds, then press r.",
    );
    let running = app
        .update_settings()
        .map(|u| format!("{} {}", u.edition, u.version))
        .unwrap_or_default();
    let mut lines = match list.selected().and_then(|i| rows.get(i)) {
        Some(r) => {
            let mut lines = vec![
                kv(&t, "Stick", r.volume.display_name()),
                kv(&t, "This computer", format!("Resilience {running}")),
            ];
            match (&r.volume.problem, &r.volume.mountpoint, &r.claim) {
                (Some(p), _, _) => lines.push(Line::from(Span::styled(
                    format!("[!!] Cannot be used: {p}."),
                    t.warn,
                ))),
                (None, None, _) => {
                    lines.push(Line::from(""));
                    lines.push(Line::from("Enter opens it and looks for an update on it."));
                }
                (None, Some(_), Some(Ok(p))) => {
                    lines.push(kv(
                        &t,
                        "The stick says",
                        format!("Resilience {} {}", p.edition, p.version),
                    ));
                    lines.push(Line::from(""));
                    lines.push(Line::from(
                        "This is what the stick claims; it is not checked yet. c checks \
                         that every file is intact and signed by a Resilience release key, \
                         without installing anything. Enter installs it (administrator \
                         password); everything is checked again first.",
                    ));
                }
                (None, Some(_), Some(Err(e))) => lines.push(Line::from(Span::styled(
                    format!("[!!] The update on this stick cannot be read: {e}."),
                    t.warn,
                ))),
                (None, Some(_), None) => {
                    lines.push(Line::from(""));
                    lines.push(Line::from(
                        "No update on this stick. An update is a folder named \
                         resilience-update at the top of the stick.",
                    ));
                }
            }
            lines
        }
        None => Vec::new(),
    };
    lines.push(Line::from(""));
    lines.push(Line::from(
        "An update installs a newer Resilience; it starts at the next restart. Your \
         files, accounts, messages and resource packs are kept, and the current version \
         stays in the boot menu.",
    ));
    details(f, &t, d, lines);
}

fn draw_generations(
    f: &mut Frame,
    app: &App,
    area: Rect,
    gens: &[recovery::Generation],
    list: &mut ListState,
) {
    let t = Theme::new(app.theme);
    if gens.is_empty() {
        let p = Paragraph::new(
            "No generations are recorded on this machine. This is normal on the live \
             system, which always starts the same way.",
        )
        .wrap(Wrap { trim: false })
        .block(block("SYSTEM GENERATIONS", &t));
        f.render_widget(p, area);
        return;
    }
    let items: Vec<ListItem> = gens
        .iter()
        .map(|g| {
            ListItem::new(format!(
                "{:>4}  {:<10}  {}",
                g.number,
                g.created
                    .map(|c| date_time(c).chars().take(10).collect::<String>())
                    .unwrap_or_else(|| "?".into()),
                generation_marks(g)
            ))
        })
        .collect();
    let [l, d] = two_panes(area);
    draw_list(f, &t, l, "SYSTEM GENERATIONS", items, list, "");
    let Some(g) = list.selected().and_then(|i| gens.get(i)) else {
        return;
    };
    let state = match (g.booted, g.running, g.default) {
        (true, _, true) => "running, default",
        (true, _, false) => "running, not default",
        (false, true, _) => "active, not restarted",
        (false, false, true) => "default at next start",
        (false, false, false) => "in the boot menu",
    };
    let mut lines = vec![
        kv(&t, "Generation", g.number.to_string()),
        kv(
            &t,
            "Created",
            g.created.map(date_time).unwrap_or_else(|| "?".into()),
        ),
        kv(
            &t,
            "Version",
            g.nixos_version.clone().unwrap_or_else(|| "?".into()),
        ),
        kv(&t, "Kernel", g.kernel.clone().unwrap_or_else(|| "?".into())),
        kv(&t, "State", state),
        Line::from(""),
    ];
    let hint = if g.default {
        "This generation starts when the computer starts. To try another one once, \
         choose it in the boot menu while the computer starts."
    } else {
        "Enter makes this generation start at every boot (administrator password). \
         To try it once instead: restart, then choose it in the boot menu (sometimes \
         under 'NixOS - All configurations')."
    };
    lines.push(Line::from(hint));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Generations contain system software and settings only, never your personal files.",
        t.dim,
    )));
    details(f, &t, d, lines);
}

fn draw_media(
    f: &mut Frame,
    app: &App,
    area: Rect,
    volumes: &[Volume],
    error: Option<&str>,
    list: &mut ListState,
    purpose: MediaPurpose,
) {
    let t = Theme::new(app.theme);
    let title = match purpose {
        MediaPurpose::InstallPacks => "COPY PACKS FROM A USB STICK",
        MediaPurpose::ReadPacks => "READ PACKS ON A USB STICK",
        MediaPurpose::Backups => "BACKUPS & USB STICKS",
        MediaPurpose::ExportKeys => "SAVE YOUR PUBLIC KEYS ON A USB STICK",
    };
    if let Some(e) = error {
        let p = Paragraph::new(format!(
            "The list of USB sticks could not be read.\n\n{e}\n\nPress r to try again."
        ))
        .wrap(Wrap { trim: false })
        .style(t.error)
        .block(block(title, &t));
        f.render_widget(p, area);
        return;
    }
    let items: Vec<ListItem> = volumes
        .iter()
        .map(|v| {
            let state = if !v.usable() {
                "unusable"
            } else if v.mountpoint.is_some() {
                "open"
            } else {
                ""
            };
            ListItem::new(format!(
                "{:<14.14} {:>9} {state}",
                v.display_name(),
                human_bytes(v.size)
            ))
        })
        .collect();
    let [l, d] = two_panes(area);
    draw_list(
        f,
        &t,
        l,
        title,
        items,
        list,
        "No USB stick or memory card found. Plug one in, wait a few seconds, then press r.",
    );
    let lines = match list.selected().and_then(|i| volumes.get(i)) {
        Some(v) => {
            let mut lines = vec![
                kv(&t, "Name", v.display_name()),
                kv(&t, "Device", v.device.clone()),
                kv(&t, "Size", human_bytes(v.size)),
                kv(&t, "File system", v.fstype.clone()),
            ];
            match &v.mountpoint {
                Some(m) => {
                    lines.push(kv(&t, "Open at", m.display().to_string()));
                    if let Some(a) = v.available {
                        lines.push(kv(&t, "Free", human_bytes(a)));
                    }
                }
                None => lines.push(kv(&t, "State", "not open")),
            }
            if v.read_only {
                lines.push(kv(&t, "Write", "protected (read-only)"));
            }
            lines.push(Line::from(""));
            match &v.problem {
                Some(p) => lines.push(Line::from(Span::styled(
                    format!("[!!] Cannot be used: {p}."),
                    t.warn,
                ))),
                None if purpose != MediaPurpose::Backups => lines.push(Line::from(
                    "Enter opens it and looks for resource packs on it (at its top level \
                     or in a folder named Resilience-packs).",
                )),
                None => lines.push(Line::from(
                    "Enter opens it: make a backup, check or restore one. Press e before \
                     unplugging it.",
                )),
            }
            lines
        }
        None => vec![Line::from(
            "Backups protect your files if this computer is lost or broken. Keep two \
             copies, on different sticks, in different places.",
        )],
    };
    details(f, &t, d, lines);
}

fn draw_library(
    f: &mut Frame,
    app: &App,
    area: Rect,
    kind: PackKind,
    items: &[LibItem],
    list: &mut ListState,
) {
    let map = kind == PackKind::Map;
    let t = Theme::new(app.theme);
    let rows: Vec<ListItem> = items
        .iter()
        .map(|i| {
            ListItem::new(format!(
                "{:<12.12} {:<46.46} {:>9}",
                i.tag(),
                i.title,
                human_bytes(i.size)
            ))
        })
        .collect();
    let [l, d] = Layout::vertical([Constraint::Min(5), Constraint::Length(6)]).areas(area);
    draw_list(
        f,
        &t,
        l,
        if map { "MAPS" } else { "OFFLINE LIBRARY" },
        rows,
        list,
        if map {
            "This computer has no map yet. Press p to choose map packs to download, or to \
             copy from a USB stick (an administrator's password is needed)."
        } else {
            "No resource packs are installed yet. Press p to choose packs to download, or to \
             copy from a USB stick (an administrator's password is needed)."
        },
    );
    let lines = match list.selected().and_then(|i| items.get(i)) {
        Some(i) => vec![
            Line::from(Span::styled(i.title.clone(), t.accent)),
            Line::from(i.description.clone()),
            Line::from(Span::styled(
                if map {
                    "Enter opens the map on it: arrows move, + and - zoom, i says what the \
                     map covers and how old it is, ? lists every key, q quits."
                } else {
                    "Enter reads it in the text browser: arrows move between links, Enter or \
                     Right follows one, Left goes back, q quits."
                },
                t.dim,
            )),
        ],
        None if map => vec![Line::from(
            "Map packs hold the streets, places and points of interest of a region. They \
             work without a network.",
        )],
        None => vec![Line::from(
            "Resource packs are large offline collections: encyclopedias, dictionaries, \
             health and practical guides. Everything here works without a network.",
        )],
    };
    details(f, &t, d, lines);
}

fn draw_pack_sets(f: &mut Frame, app: &App, area: Rect, list: &mut ListState) {
    let t = Theme::new(app.theme);
    // The packs screen is right below this one.
    let Some(s) = app.stack.iter().rev().find_map(|v| match v {
        View::Packs { s, .. } => Some(&**s),
        _ => None,
    }) else {
        return;
    };
    let summary = |set: &resilience_core::packs::PackSet| {
        let mut add = 0u64;
        let mut missing = 0;
        for id in &set.packs {
            if let Some(i) = s.cfg.packs.iter().position(|p| &p.id == id) {
                match s.inv.states[i] {
                    PackState::Installed => {}
                    PackState::Partial(n) => {
                        add += s.cfg.packs[i].size - n;
                        missing += 1;
                    }
                    PackState::Absent => {
                        add += s.cfg.packs[i].size;
                        missing += 1;
                    }
                }
            }
        }
        (missing, add)
    };
    let items: Vec<ListItem> = s
        .cfg
        .sets
        .iter()
        .map(|set| {
            let (missing, add) = summary(set);
            let state = if missing == 0 {
                "all installed".to_string()
            } else {
                format!("{missing} to get, {}", human_bytes(add))
            };
            ListItem::new(format!("{:<34.34} {state}", set.title))
        })
        .collect();
    let [l, d] = Layout::vertical([Constraint::Min(5), Constraint::Length(9)]).areas(area);
    draw_list(f, &t, l, "SUGGESTED SETS", items, list, "");
    let Some(set) = list.selected().and_then(|i| s.cfg.sets.get(i)) else {
        return;
    };
    let titles: Vec<&str> = set
        .packs
        .iter()
        .filter_map(|id| s.cfg.pack(id).map(|p| p.title.as_str()))
        .collect();
    let lines = vec![
        Line::from(Span::styled(set.description.clone(), t.base)),
        Line::from(Span::styled(titles.join("; "), t.dim)),
        Line::from(Span::styled(
            "Enter ticks every pack of this set that is not installed. Nothing is \
             downloaded until you press d (or u for a USB stick).",
            t.dim,
        )),
    ];
    details(f, &t, d, lines);
}

fn draw_packs(f: &mut Frame, app: &App, area: Rect, s: &PacksScreen, list: &mut ListState) {
    let t = Theme::new(app.theme);
    let [top, l, d] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(5),
        Constraint::Length(6),
    ])
    .areas(area);
    let plan = s.plan();
    let mut summary = vec![Span::styled(
        format!(
            "Add {} ({})  Remove {}  ",
            plan.get.len(),
            human_bytes(plan.add_bytes),
            plan.remove.len()
        ),
        t.base,
    )];
    if let Some(free) = s.free {
        let after = free + plan.remove_bytes;
        let style = if plan.add_bytes > after {
            t.error
        } else {
            t.dim
        };
        summary.push(Span::styled(format!("Free {}", human_bytes(free)), style));
        if plan.add_bytes > after {
            summary.push(Span::styled("  [!!] not enough space", t.error));
        }
    }
    f.render_widget(Paragraph::new(Line::from(summary)), top);

    let rows = s.rows();
    let inner = l.width.saturating_sub(4) as usize;
    // "[x] " + title + " " + size (9) + "  " + state (up to 11).
    let title_w = inner.saturating_sub(4 + 1 + 9 + 2 + 11).clamp(16, 60);
    let items: Vec<ListItem> = rows
        .iter()
        .map(|r| match *r {
            PackRow::Header(c) => ListItem::new(Span::styled(
                format!("-- {} --", c.label().to_uppercase()),
                t.accent,
            )),
            PackRow::OthersHeader => ListItem::new(Span::styled(
                "-- OTHER FILES (not in the catalog) --",
                t.accent,
            )),
            PackRow::Pack(i) => {
                let p = &s.cfg.packs[i];
                let (mark, state) = s.row_state(*r);
                ListItem::new(format!(
                    "{mark} {:<title_w$.title_w$} {:>9}  {state}",
                    p.title,
                    human_bytes(p.size)
                ))
            }
            PackRow::Other(j) => {
                let o = &s.inv.others[j];
                let (mark, state) = s.row_state(*r);
                ListItem::new(format!(
                    "{mark} {:<title_w$.title_w$} {:>9}  {state}",
                    o.file,
                    human_bytes(o.size)
                ))
            }
        })
        .collect();
    let title = match &s.language {
        Some(lang) => format!("RESOURCE PACKS (language: {lang}; f for all)"),
        None => "RESOURCE PACKS (all languages; f to choose)".to_string(),
    };
    draw_list(
        f,
        &t,
        l,
        &title,
        items,
        list,
        "No resource packs are offered on this system.",
    );
    let lines = match list.selected().and_then(|i| rows.get(i)) {
        Some(PackRow::Pack(i)) => {
            let p = &s.cfg.packs[*i];
            vec![
                Line::from(Span::styled(p.description.clone(), t.base)),
                Line::from(Span::styled(
                    format!("Language: {}  License: {}", p.language, p.license),
                    t.dim,
                )),
                Line::from(Span::styled(
                    "Space ticks it. [+] will be added, [-] will be removed, [*] is installed.",
                    t.dim,
                )),
            ]
        }
        Some(PackRow::Other(_)) => vec![Line::from(
            "A file that is not in this system's catalog, usually an older version. It \
             can still be read; tick it to remove it and free the space.",
        )],
        _ => vec![Line::from(
            "Packs are shared by every account. Tick what you want with Space, then press \
             d to download (Internet needed) or u to copy from a USB stick.",
        )],
    };
    details(f, &t, d, lines);
}

fn draw_wifi(
    f: &mut Frame,
    app: &App,
    area: Rect,
    overview: Option<&Overview>,
    error: Option<&str>,
    list: &mut ListState,
) {
    let t = Theme::new(app.theme);
    let message = match (overview, error) {
        (_, Some(e)) => Some((
            format!("Wi-Fi information could not be read.\n\n{e}\n\nPress r to try again."),
            t.error,
        )),
        (Some(o), None) if o.devices == 0 => Some((
            "No Wi-Fi adapter was found. If this computer has one, it may need \
             firmware that is not included, or a hardware switch may be off.\n\n\
             A network cable works without any setup."
                .to_string(),
            t.warn,
        )),
        (Some(o), None) if !o.radio_on => Some((
            "Wi-Fi is switched off. Press w to switch it on.".to_string(),
            t.warn,
        )),
        _ => None,
    };
    if let Some((text, style)) = message {
        let p = Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .style(style)
            .block(block("WI-FI NETWORKS", &t));
        f.render_widget(p, area);
        return;
    }
    let nets: &[Network] = overview.map_or(&[], |o| o.networks.as_slice());
    let items: Vec<ListItem> = nets
        .iter()
        .map(|n| {
            // Fits the 80-column list pane: "*Name            55% WPA2   saved".
            ListItem::new(format!(
                "{}{:<16.16} {:>3}% {:<6} {}",
                if n.in_use { "*" } else { " " },
                n.ssid,
                n.signal,
                n.security.label(),
                if n.saved.is_some() { "saved" } else { "" },
            ))
        })
        .collect();
    let [l, d] = two_panes(area);
    draw_list(
        f,
        &t,
        l,
        "WI-FI NETWORKS",
        items,
        list,
        "No networks found yet. Scanning takes a few seconds; press r to look again.",
    );
    let lines = match list.selected().and_then(|i| nets.get(i)) {
        Some(n) => {
            let mut lines = vec![
                kv(&t, "Network", n.ssid.clone()),
                kv(
                    &t,
                    "Signal",
                    format!("{}% {}", n.signal, wifi::bars(n.signal)),
                ),
                kv(&t, "Security", n.security.label().to_string()),
                kv(
                    &t,
                    "State",
                    if n.in_use {
                        "connected"
                    } else if n.saved.is_some() {
                        "saved, not connected"
                    } else {
                        "not saved"
                    },
                ),
                Line::from(""),
            ];
            let hint = if n.in_use {
                "d disconnects. f forgets the network and its password.".to_string()
            } else if n.saved.is_some() {
                "Enter connects with the saved password. f forgets it.".to_string()
            } else if let Some(p) = n.problem() {
                return details(
                    f,
                    &t,
                    d,
                    [
                        lines,
                        vec![Line::from(Span::styled(format!("[!!] {p}."), t.warn))],
                    ]
                    .concat(),
                );
            } else if n.security == wifi::Security::Open {
                "Enter joins it. Open network: anyone nearby can read unencrypted traffic."
                    .to_string()
            } else {
                "Enter joins it. You will be asked for the network's password.".to_string()
            };
            lines.push(Line::from(hint));
            lines
        }
        None => vec![Line::from(
            "Networks you join are saved for your account only. Other accounts on \
             this computer do not see them.",
        )],
    };
    details(f, &t, d, lines);
}

fn draw_backups(
    f: &mut Frame,
    app: &App,
    area: Rect,
    volume: &Volume,
    mount: &std::path::Path,
    files: &[BackupFile],
    list: &mut ListState,
) {
    let t = Theme::new(app.theme);
    let mut items = vec![ListItem::new("+ Make a new backup of my files")];
    items.extend(files.iter().map(|b| {
        ListItem::new(format!(
            "{}  {:>10}",
            backup::human_stamp(&b.stamp),
            human_bytes(b.size)
        ))
    }));
    let title = format!("BACKUPS ON {}", volume.display_name().to_uppercase());
    let [l, d] = two_panes(area);
    draw_list(f, &t, l, &title, items, list, "");
    let lines = match list.selected().unwrap_or(0).checked_sub(1) {
        None => {
            let mut lines = vec![
                Line::from(
                    "Saves your whole home folder (notes, documents, settings) in one \
                     encrypted file on this stick. You choose its passphrase.",
                ),
                Line::from(""),
            ];
            if files.is_empty() {
                lines.push(Line::from(Span::styled(
                    "No backups on this stick yet.",
                    t.dim,
                )));
            }
            lines.push(kv(&t, "Stick", mount.display().to_string()));
            lines
        }
        Some(i) => match files.get(i) {
            Some(b) => vec![
                kv(&t, "Made", backup::human_stamp(&b.stamp)),
                kv(&t, "Size", human_bytes(b.size)),
                kv(&t, "File", b.name.clone()),
                kv(
                    &t,
                    "Checksum",
                    if b.has_checksum { "yes" } else { "missing" },
                ),
                Line::from(""),
                Line::from(
                    "Enter restores it into a new folder in Restored; nothing is overwritten. \
                     c checks that the copy is intact.",
                ),
            ],
            None => Vec::new(),
        },
    };
    details(f, &t, d, lines);
}

fn draw_files(
    f: &mut Frame,
    app: &App,
    area: Rect,
    idx: &resilience_core::files::Index,
    query: &str,
    list: &mut ListState,
    pick: Option<crate::app::Pick>,
) {
    let t = Theme::new(app.theme);
    let title = pick
        .map(|p| p.title())
        .unwrap_or("FIND FILES (home folder and open USB sticks)");
    let [input, results] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).areas(area);
    let cursor = if (app.tick / 2).is_multiple_of(2) {
        "█"
    } else {
        " "
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(query.to_string(), t.accent),
            Span::styled(cursor, t.base),
        ]))
        .block(block(title, &t)),
        input,
    );
    let hits = resilience_core::files::search(idx, query);
    let w = results.width.saturating_sub(4 + 11) as usize;
    let items: Vec<ListItem> = hits
        .iter()
        .map(|e| {
            // Keep the end of long paths: the file name matters most.
            let n = e.shown.chars().count();
            let shown: String = if n > w {
                let tail: String = e.shown.chars().skip(n - w + 3).collect();
                format!("...{tail}")
            } else {
                e.shown.clone()
            };
            ListItem::new(format!("{shown:<w$} {:>10}", human_bytes(e.size)))
        })
        .collect();
    let empty = if query.trim().is_empty() {
        format!(
            "Type part of a file name. {} files can be searched.",
            idx.entries.len()
        )
    } else {
        "No file name matches.".to_string()
    };
    draw_list(
        f,
        &t,
        results,
        &format!("FILES ({})", hits.len()),
        items,
        list,
        &empty,
    );
}

fn draw_table(f: &mut Frame, app: &App, area: Rect, tbl: &crate::app::Table) {
    let t = Theme::new(app.theme);
    let inner_w = area.width.saturating_sub(2) as usize;
    let inner_h = area.height.saturating_sub(2) as usize;
    let cols = tbl.widths.len();
    let widths = &tbl.widths;
    let fmt_row = |r: &Vec<String>| -> String {
        let mut line = String::new();
        for (c, w) in widths.iter().enumerate().skip(tbl.col) {
            let cell = r.get(c).map(String::as_str).unwrap_or("");
            let cell: String = cell
                .chars()
                .map(|ch| if ch.is_control() { ' ' } else { ch })
                .take(*w)
                .collect();
            line.push_str(&format!("{cell:<w$} | "));
            if line.chars().count() > inner_w {
                break;
            }
        }
        line.chars().take(inner_w).collect()
    };
    let mut lines = Vec::new();
    if let Some(h) = tbl.rows.first() {
        lines.push(Line::from(Span::styled(fmt_row(h), t.accent)));
    }
    for r in tbl
        .rows
        .iter()
        .skip(1 + tbl.row)
        .take(inner_h.saturating_sub(1))
    {
        lines.push(Line::from(fmt_row(r)));
    }
    let pos = format!(
        " row {}/{}  column {}/{} ",
        (tbl.row + 1).min(tbl.rows.len().saturating_sub(1)),
        tbl.rows.len().saturating_sub(1),
        (tbl.col + 1).min(cols),
        cols
    );
    let b =
        block(&tbl.title, &t).title_bottom(Line::from(Span::styled(pos, t.dim)).right_aligned());
    f.render_widget(Paragraph::new(lines).style(t.base).block(b), area);
}

fn draw_search(
    f: &mut Frame,
    app: &App,
    area: Rect,
    query: &str,
    hits: &[SearchHit],
    list: &mut ListState,
) {
    let t = Theme::new(app.theme);
    let [input, results] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).areas(area);
    let cursor = if (app.tick / 2).is_multiple_of(2) {
        "█"
    } else {
        " "
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(query.to_string(), t.accent),
            Span::styled(cursor, t.base),
        ]))
        .block(block("SEARCH", &t)),
        input,
    );
    let items: Vec<ListItem> = hits
        .iter()
        .map(|h| {
            ListItem::new(match h {
                SearchHit::Category(c) => format!("[MENU] {}", c.title()),
                SearchHit::Entry(e @ Entry::Builtin(_)) => format!("[TOOL] {}", app.entry_name(*e)),
                SearchHit::Entry(e @ Entry::App(_)) => format!("[APP ] {}", app.entry_name(*e)),
                SearchHit::Page(p) => format!("[DOC ] {}", p.title),
            })
        })
        .collect();
    let empty = if query.trim().is_empty() {
        "Type a word: an application, a topic, a setting..."
    } else {
        "No matches."
    };
    draw_list(
        f,
        &t,
        results,
        &format!("RESULTS ({})", hits.len()),
        items,
        list,
        empty,
    );
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

fn draw_modal(f: &mut Frame, app: &App, area: Rect) {
    let t = Theme::new(app.theme);
    let Some(m) = &app.modal else { return };
    let (title, mut lines, style) = match m {
        Modal::Message { title, body, error } => {
            let mut l: Vec<Line> = body.lines().map(|s| Line::from(s.to_string())).collect();
            l.push(Line::from(""));
            l.push(Line::from(Span::styled("[ OK ]  (Enter)", t.selected)).centered());
            (title.as_str(), l, if *error { t.error } else { t.base })
        }
        Modal::Confirm {
            title,
            body,
            confirm,
            yes_focused,
            ..
        } => {
            let mut l: Vec<Line> = body.lines().map(|s| Line::from(s.to_string())).collect();
            l.push(Line::from(""));
            let (ys, ns) = if *yes_focused {
                (t.selected, t.base)
            } else {
                (t.base, t.selected)
            };
            l.push(
                Line::from(vec![
                    Span::styled(format!("[ {confirm} (y) ]"), ys),
                    Span::raw("   "),
                    Span::styled("[ Cancel (n) ]", ns),
                ])
                .centered(),
            );
            (title.as_str(), l, t.warn)
        }
        Modal::NewNote { value, error } => {
            let mut l = vec![
                Line::from("Name for the new note:"),
                Line::from(vec![
                    Span::styled(format!("> {value}"), t.accent),
                    Span::styled("█", t.base),
                ]),
                Line::from(""),
            ];
            match error {
                Some(e) => l.push(Line::from(Span::styled(format!("[!!] {e}"), t.error))),
                None => l.push(Line::from(Span::styled(
                    "Letters, digits, spaces, - _ . | Enter create, Esc cancel",
                    t.dim,
                ))),
            }
            ("NEW NOTE", l, t.base)
        }
        Modal::WifiPassword {
            net, value, error, ..
        } => {
            let mut l = vec![
                Line::from(format!("Password for the Wi-Fi network '{}':", net.ssid)),
                Line::from(vec![
                    Span::styled(format!("> {}", "*".repeat(value.chars().count())), t.accent),
                    Span::styled("█", t.base),
                ]),
                Line::from(""),
            ];
            match error {
                Some(e) => l.push(Line::from(Span::styled(format!("[!!] {e}"), t.error))),
                None => l.push(Line::from(Span::styled(
                    "Usually on a label on the router | Enter connect, Esc cancel",
                    t.dim,
                ))),
            }
            ("WI-FI PASSWORD", l, t.base)
        }
        Modal::ManPage { value, error } => {
            let mut l = vec![
                Line::from("Command name, or words to search the manuals for:"),
                Line::from(vec![
                    Span::styled(format!("> {value}"), t.accent),
                    Span::styled("█", t.base),
                ]),
                Line::from(""),
            ];
            match error {
                Some(e) => l.push(Line::from(Span::styled(format!("[!!] {e}"), t.error))),
                None => l.push(Line::from(Span::styled(
                    "e.g. ls, 5 passwd, copy files | In a page: q quits, / searches",
                    t.dim,
                ))),
            }
            ("MANUAL PAGES", l, t.base)
        }
        Modal::PackSearch { item, value } => {
            let l = vec![
                Line::from(format!("Search in '{}':", item.title)),
                Line::from(vec![
                    Span::styled(format!("> {value}"), t.accent),
                    Span::styled("█", t.base),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "Words to look for in the full text | Enter search, Esc cancel",
                    t.dim,
                )),
            ];
            ("SEARCH", l, t.base)
        }
        Modal::Recipients {
            target,
            rows,
            ticked,
            sel,
            error,
        } => {
            let what = match target {
                crate::app::EncryptTarget::File(p) => format!(
                    "Encrypt '{}' for:",
                    p.file_name()
                        .map(|n| n.to_string_lossy())
                        .unwrap_or_default()
                ),
                crate::app::EncryptTarget::Message => "Encrypt the message for:".to_string(),
            };
            let mut l = vec![Line::from(what), Line::from("")];
            for (i, (r, on)) in rows.iter().zip(ticked).enumerate() {
                let name = match r {
                    crate::app::Recipient::Passphrase => {
                        "A passphrase (you type it; give it to the reader)".to_string()
                    }
                    crate::app::Recipient::Me => "Me (my encryption key)".to_string(),
                    crate::app::Recipient::Contact(n) => resilience_core::printable(n),
                    crate::app::Recipient::OpenPgp { name, .. } => {
                        format!("OpenPGP: {}", resilience_core::printable(name))
                    }
                };
                let mark = if *on { "[x]" } else { "[ ]" };
                let style = if i == *sel { t.selected } else { t.base };
                l.push(Line::from(Span::styled(format!(" {mark} {name}"), style)));
            }
            l.push(Line::from(""));
            match error {
                Some(e) => l.push(Line::from(Span::styled(format!("[!!] {e}"), t.error))),
                None => l.push(Line::from(Span::styled(
                    "Space tick | Enter encrypt | Esc cancel | Several people: tick each",
                    t.dim,
                ))),
            }
            ("ENCRYPT FOR", l, t.base)
        }
        Modal::ChatInput { kind, value, error } => {
            let (title, ask, hint) = match kind {
                ChatInput::Address => (
                    "WRITE TO AN ADDRESS",
                    "Address of the person (they find it under 'm' in Local messages):".to_string(),
                    "32 characters, spaces allowed | Enter continue, Esc cancel",
                ),
                ChatInput::Text { peer } => (
                    "WRITE A MESSAGE",
                    format!("Message for {}:", messages::short_address(peer)),
                    "Enter send, Ctrl-N new line, Esc cancel | Kept and retried until delivered",
                ),
                ChatInput::FileText { peer, path, size } => (
                    "SEND A FILE",
                    format!(
                        "File '{}' ({}) for {}. A message with it (optional):",
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
                    "Enter send, Ctrl-N new line, Esc cancel | A copy is sent; it is retried until delivered",
                ),
                ChatInput::ContactName { .. } => (
                    "NAME THIS PERSON",
                    "Your own name for this address (empty: no name):".to_string(),
                    "Only you see it | Enter save, Esc cancel",
                ),
            };
            let mut l = vec![Line::from(ask)];
            // One screen line per typed line; the cursor after the last.
            let typed: Vec<&str> = value.split('\n').collect();
            for (i, part) in typed.iter().enumerate() {
                let mut spans = vec![Span::styled(
                    format!("{}{part}", if i == 0 { "> " } else { "  " }),
                    t.accent,
                )];
                if i + 1 == typed.len() {
                    spans.push(Span::styled("█", t.base));
                }
                l.push(Line::from(spans));
            }
            l.push(Line::from(""));
            match error {
                Some(e) => l.push(Line::from(Span::styled(format!("[!!] {e}"), t.error))),
                None => l.push(Line::from(Span::styled(hint, t.dim))),
            }
            (title, l, t.base)
        }
        Modal::ShareInput { kind, value, error } => {
            let (title, ask, hint) = match kind {
                ShareInput::Address => (
                    "ADDRESS OF A COMPUTER",
                    "Address of the computer that shares (for example 192.168.1.20, or \
                     fe80::1%bat0 on the mesh):",
                    "A computer on a network this computer is on | Enter open, Esc cancel",
                ),
                ShareInput::Name => (
                    "NAME OTHERS SEE",
                    "The name other computers will see (a place, not a person):",
                    "Up to 32 letters, digits, spaces or dashes | Enter save, Esc cancel",
                ),
            };
            let mut l = vec![
                Line::from(ask),
                Line::from(vec![
                    Span::styled(format!("> {value}"), t.accent),
                    Span::styled("█", t.base),
                ]),
                Line::from(""),
            ];
            match error {
                Some(e) => l.push(Line::from(Span::styled(format!("[!!] {e}"), t.error))),
                None => l.push(Line::from(Span::styled(hint, t.dim))),
            }
            (title, l, t.base)
        }
        Modal::ExpectedSum { file, value, error } => {
            let mut l = vec![
                Line::from(format!(
                    "Checksum that '{}' should have:",
                    file.file_name()
                        .map(|n| n.to_string_lossy())
                        .unwrap_or_default()
                )),
                Line::from(vec![
                    Span::styled(format!("> {value}"), t.accent),
                    Span::styled("█", t.base),
                ]),
                Line::from(""),
            ];
            match error {
                Some(e) => l.push(Line::from(Span::styled(format!("[!!] {e}"), t.error))),
                None => l.push(Line::from(Span::styled(
                    "SHA-256, SHA-512, SHA-1 or MD5, as published | Enter compare, Esc cancel",
                    t.dim,
                ))),
            }
            ("COMPARE WITH A CHECKSUM", l, t.base)
        }
        Modal::InstallText {
            field,
            value,
            error,
        } => {
            let (title, prompt, hint): (&str, String, &str) = match field {
                InstallInput::Name(_) => (
                    "ACCOUNT NAME",
                    "Login name (lowercase letters, digits, - and _):".into(),
                    "Enter OK, Esc cancel",
                ),
                InstallInput::FullName(_) => (
                    "FULL NAME",
                    "Full name or description (may stay empty):".into(),
                    "Enter OK, Esc cancel",
                ),
                InstallInput::Confirm => (
                    "ERASE THIS DISK AND INSTALL?",
                    format!(
                        "{}\n\nTo go on, type the disk's name ({}):",
                        app.install_summary(),
                        app.install_form()
                            .and_then(|fm| fm.disk.as_ref())
                            .map(|d| d.name().to_string())
                            .unwrap_or_default()
                    ),
                    "Enter install, Esc go back",
                ),
            };
            let mut l: Vec<Line> = prompt.lines().map(|x| Line::from(x.to_string())).collect();
            l.push(Line::from(vec![
                Span::styled(format!("> {value}"), t.accent),
                Span::styled("█", t.base),
            ]));
            l.push(Line::from(""));
            match error {
                Some(e) => l.push(Line::from(Span::styled(format!("[!!] {e}"), t.error))),
                None => l.push(Line::from(Span::styled(hint, t.dim))),
            }
            let style = if matches!(field, InstallInput::Confirm) {
                t.warn
            } else {
                t.base
            };
            (title, l, style)
        }
        Modal::SetTime { value, zone, error } => {
            let mut l = vec![
                Line::from(format!(
                    "The clock shows {} (time zone {zone}).",
                    date_time(SystemTime::now())
                )),
                Line::from(""),
                Line::from("Correct date and time:"),
                Line::from(vec![
                    Span::styled(format!("> {value}"), t.accent),
                    Span::styled("█", t.base),
                ]),
                Line::from(""),
            ];
            match error {
                Some(e) => l.push(Line::from(Span::styled(format!("[!!] {e}"), t.error))),
                None => l.push(Line::from(Span::styled(
                    "YYYY-MM-DD HH:MM | Enter continue, Esc cancel",
                    t.dim,
                ))),
            }
            ("DATE & TIME", l, t.base)
        }
    };
    let width = 64u16;
    lines = wrap(lines, width as usize - 4);
    // A long message being typed: keep its end (the cursor) on the screen.
    let room = usize::from(area.height.saturating_sub(4)).max(4);
    if matches!(m, Modal::ChatInput { .. }) && lines.len() > room {
        let cut = lines.len() - room;
        lines.drain(1..=cut);
    }
    let r = centered(area, width, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    let b = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(style)
        .title(Span::styled(format!(" {title} "), t.title))
        .style(Style::new().patch(t.base));
    f.render_widget(Paragraph::new(lines).style(t.base).block(b), r);
}

// ----- local messages (ADR 0018) ---------------------------------------------

fn chat_status_line(t: &Theme, status: &Status) -> Line<'static> {
    match status {
        Status::Running { address, .. } => Line::from(vec![
            Span::styled("[OK] ", t.ok),
            Span::raw(format!(
                "Messaging is running. My address: {} (m shows all of it)",
                messages::short_address(address)
            )),
        ]),
        Status::Stopped { .. } => Line::from(Span::styled(
            "[!!] Messaging is not running for your account: nothing is sent or received. \
             Log out and in again.",
            t.warn,
        )),
    }
}

fn draw_chats(
    f: &mut Frame,
    app: &App,
    area: Rect,
    rows: &[ChatRow],
    status: &Status,
    list: &mut ListState,
) {
    let t = Theme::new(app.theme);
    let now = crate::app::chat_now();
    let items: Vec<ListItem> = rows
        .iter()
        .map(|r| {
            let c = &r.conv;
            let flag = if c.failed > 0 {
                "[!!]"
            } else if c.pending > 0 {
                "[..]"
            } else {
                "    "
            };
            let who = if c.last_direction == Direction::Out {
                "You: "
            } else {
                ""
            };
            ListItem::new(format!(
                "{flag} {:<22.22} {:>11}  {who}{}",
                r.name.display(&c.peer),
                messages::ago(c.last_time, now),
                c.last_text
            ))
        })
        .collect();
    let [l, d] = Layout::vertical([Constraint::Min(5), Constraint::Length(6)]).areas(area);
    draw_list(
        f,
        &t,
        l,
        "LOCAL MESSAGES",
        items,
        list,
        "No messages yet. Press p to see the people on your local network, or n to write \
         to an address someone gave you. Press m for your own address.",
    );
    let mut lines = vec![chat_status_line(&t, status)];
    // Store-and-forward (ADR 0019): whether people who are away can be
    // written to, a property of the network rather than of a conversation.
    if let Status::Running { store, .. } = status {
        lines.push(Line::from(Span::styled(
            if *store {
                "Messages for people who are away wait in a message store on this network \
                 until their computer collects them."
            } else {
                "No message store on this network: a message arrives only when both \
                 computers are on at the same time."
            },
            t.dim,
        )));
    }
    match list.selected().and_then(|i| rows.get(i)) {
        Some(r) => {
            let c = &r.conv;
            lines.push(Line::from(format!(
                "{} messages with {}. {} waiting or sent without proof, {} failed.",
                c.count,
                messages::format_address(&c.peer),
                c.pending,
                c.failed
            )));
            if let Name::Claimed(_) = r.name {
                lines.push(Line::from(Span::styled(
                    "A name in quotes with ? is what that address says about itself: anyone \
                     can claim any name.",
                    t.dim,
                )));
            }
        }
        None => lines.push(Line::from(Span::styled(
            "Messages travel on the local network only, without the Internet, between \
             computers that have local messaging.",
            t.dim,
        ))),
    }
    details(f, &t, d, lines);
}

fn draw_nearby(f: &mut Frame, app: &App, area: Rect, rows: &[NearbyRow], list: &mut ListState) {
    let t = Theme::new(app.theme);
    let now = crate::app::chat_now();
    let items: Vec<ListItem> = rows
        .iter()
        .map(|r| {
            ListItem::new(format!(
                "{:<26.26} {}  heard {}",
                r.name.display(&r.peer.address),
                messages::format_address(&r.peer.address),
                messages::ago(r.peer.heard, now)
            ))
        })
        .collect();
    let [l, d] = Layout::vertical([Constraint::Min(5), Constraint::Length(5)]).areas(area);
    draw_list(
        f,
        &t,
        l,
        "PEOPLE NEARBY",
        items,
        list,
        "Nobody has been heard on the local network yet. Other computers appear here a \
         little after they join the same network, if they have local messaging.",
    );
    details(
        f,
        &t,
        d,
        vec![
            Line::from(
                "Everyone heard on the local network. A name in quotes with ? is only what \
                 that address says about itself.",
            ),
            Line::from(Span::styled(
                "Compare the address with the person (in person or by voice), then press a \
                 to give it a name of your own.",
                t.dim,
            )),
        ],
    );
}

fn message_item(r: &Record, t: &Theme, now: u64, width: usize) -> ListItem<'static> {
    let (who, style) = match (r.direction, r.state) {
        (Direction::In, _) if r.verified => ("Them", t.accent),
        (Direction::In, _) => ("Them", t.warn),
        (_, State::Failed) => ("You", t.error),
        (_, State::Delivered) => ("You", t.ok),
        _ => ("You", t.dim),
    };
    let mut head = format!("{who} [{}] {}", r.marker(), messages::ago(r.created, now));
    if !r.detail.is_empty() && r.state != State::Delivered {
        head.push_str(&format!(" - {}", messages::clean_line(&r.detail, 80)));
    }
    let mut lines = vec![Line::from(Span::styled(head, style))];
    let body: Vec<Line<'static>> = messages::clean_text(&r.text, 200)
        .into_iter()
        .map(|l| Line::from(format!("  {l}")))
        .collect();
    lines.extend(wrap(body, width.max(20)));
    if let Some(file) = &r.file {
        let mut l = format!(
            "  [file] {} ({})",
            file.shown(),
            messages::size_text(file.size)
        );
        if r.direction == Direction::In {
            l.push_str(if file.data.is_some() {
                "  o open, s save"
            } else {
                "  not kept"
            });
        }
        lines.push(Line::from(Span::styled(l, t.accent)));
    }
    ListItem::new(Text::from(wrap(lines, width.max(20))))
}

#[allow(clippy::too_many_arguments)]
fn draw_chat(
    f: &mut Frame,
    app: &App,
    area: Rect,
    peer: &str,
    name: &Name,
    records: &[Record],
    list: &mut ListState,
) {
    let t = Theme::new(app.theme);
    let now = crate::app::chat_now();
    let width = usize::from(area.width.saturating_sub(6));
    let items: Vec<ListItem> = records
        .iter()
        .map(|r| message_item(r, &t, now, width))
        .collect();
    let title = format!(
        "MESSAGES WITH {} ({})",
        name.display(peer),
        messages::short_address(peer)
    );
    let [l, d] = Layout::vertical([Constraint::Min(5), Constraint::Length(5)]).areas(area);
    draw_list(
        f,
        &t,
        l,
        &title,
        items,
        list,
        "No messages with this person yet. Press w to write one.",
    );
    details(
        f,
        &t,
        d,
        vec![
            Line::from(
                "waiting: kept here and tried again | sent: no proof yet | delivered: their \
                 computer confirmed it has the message | FAILED: given up.",
            ),
            Line::from(
                "kept on the network: a copy waits in a message store for them | stopped, may \
                 still arrive: given up here, but a stored copy may still reach them.",
            ),
            Line::from(Span::styled(
                "NOT VERIFIED: the message could not be checked against this address's key.",
                t.dim,
            )),
        ],
    );
}

fn draw_community(
    f: &mut Frame,
    app: &App,
    area: Rect,
    rows: &[CommunityRow],
    list: &mut ListState,
) {
    let t = Theme::new(app.theme);
    let now = crate::app::chat_now();
    let items: Vec<ListItem> = rows
        .iter()
        .map(|r| match r {
            CommunityRow::Sharing(s) => ListItem::new(Span::styled(
                format!(
                    "[ON] Sharing {} on {} as \"{}\" - Enter to stop",
                    s.services.join(", "),
                    s.interface,
                    s.name
                ),
                t.warn,
            )),
            CommunityRow::Share => ListItem::new("Share this computer's resources..."),
            CommunityRow::Address => ListItem::new("Type the address of a computer..."),
            CommunityRow::Hotspot { on: Some(what) } => ListItem::new(Span::styled(
                format!("[ON] Hotspot {what} - Enter to turn off"),
                t.warn,
            )),
            CommunityRow::Hotspot { on: None } => {
                ListItem::new("Create a network: hotspot (this computer as Wi-Fi)...")
            }
            CommunityRow::Mesh { on: true } => ListItem::new(Span::styled(
                "[ON] Mesh network - Enter to turn off",
                t.warn,
            )),
            CommunityRow::Mesh { on: false } => {
                ListItem::new("Create a network: mesh (Wi-Fi without a router)...")
            }
            CommunityRow::Machine { heard, peer } => {
                let what: Vec<&str> = heard.services.iter().map(|s| s.id()).collect();
                let reach = match peer {
                    Some(p) => p.display(),
                    None => "not reachable from here".into(),
                };
                ListItem::new(format!(
                    "\"{}\"?  {}  {}  heard {}",
                    heard.name,
                    what.join(", "),
                    reach,
                    messages::ago(heard.heard, now)
                ))
            }
        })
        .collect();
    let [l, d] = Layout::vertical([Constraint::Min(5), Constraint::Length(6)]).areas(area);
    draw_list(f, &t, l, "COMMUNITY NETWORK", items, list, "");
    let heard = rows
        .iter()
        .filter(|r| matches!(r, CommunityRow::Machine { .. }))
        .count();
    let mut lines = vec![Line::from(if heard == 0 {
        "No computer sharing has been heard yet. They appear here a little after \
         joining the same network (Wi-Fi, cable or mesh), if they have local messaging."
            .to_string()
    } else {
        format!(
            "{heard} computer(s) sharing heard on the local networks. A name in quotes \
             with ? is only what that computer says about itself."
        )
    })];
    lines.push(Line::from(Span::styled(
        "Packs you get are checked against this system's catalog. Pages and files are \
         not checked: they are as trustworthy as whoever runs that computer.",
        t.dim,
    )));
    details(f, &t, d, lines);
}

#[allow(clippy::too_many_arguments)]
fn draw_peer_menu(
    f: &mut Frame,
    app: &App,
    area: Rect,
    name: &str,
    peer: &resilience_core::share::Peer,
    actions: &[PeerAction],
    list: &mut ListState,
) {
    let t = Theme::new(app.theme);
    let items: Vec<ListItem> = actions.iter().map(|a| ListItem::new(a.name())).collect();
    let [l, d] = Layout::vertical([Constraint::Min(5), Constraint::Length(5)]).areas(area);
    draw_list(f, &t, l, "NEARBY COMPUTER", items, list, "");
    details(
        f,
        &t,
        d,
        vec![
            Line::from(format!(
                "{} at {}",
                resilience_core::printable(name),
                peer.display()
            )),
            Line::from(Span::styled(
                "Pages open in the text browser; press q there to come back. Anyone on \
                 this network can read them too.",
                t.dim,
            )),
        ],
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_peer_packs(
    f: &mut Frame,
    app: &App,
    area: Rect,
    peer: &resilience_core::share::Peer,
    rows: &[PeerPack],
    ticked: &[bool],
    list: &mut ListState,
) {
    let t = Theme::new(app.theme);
    let items: Vec<ListItem> = rows
        .iter()
        .zip(ticked)
        .map(|(r, on)| {
            let mark = if r.installed {
                "[*]"
            } else if *on {
                "[+]"
            } else {
                "[ ]"
            };
            ListItem::new(format!(
                "{mark} {:<50.50} {:>10}",
                r.title,
                human_bytes(r.size)
            ))
        })
        .collect();
    let total: u64 = rows
        .iter()
        .zip(ticked)
        .filter(|(_, on)| **on)
        .map(|(r, _)| r.size)
        .sum();
    let [l, d] = Layout::vertical([Constraint::Min(5), Constraint::Length(5)]).areas(area);
    draw_list(
        f,
        &t,
        l,
        "PACKS FROM A NEARBY COMPUTER",
        items,
        list,
        "This computer offers no pack that is in this system's catalog.",
    );
    details(
        f,
        &t,
        d,
        vec![
            Line::from(format!(
                "From {}. Ticked: {}. [*] already installed.",
                peer.display(),
                human_bytes(total)
            )),
            Line::from(Span::styled(
                "Each pack is checked against this system's catalog before it is installed.",
                t.dim,
            )),
        ],
    );
}

fn draw_share_setup(f: &mut Frame, app: &App, area: Rect, form: &ShareForm, list: &mut ListState) {
    let t = Theme::new(app.theme);
    let iface = form
        .interfaces
        .get(form.iface)
        .map(String::as_str)
        .unwrap_or("-");
    let mut items = vec![
        ListItem::new(format!("Network:  {iface}   (Enter: next network)")),
        ListItem::new(format!("Name others see:  {}", form.name)),
    ];
    for (s, on) in resilience_core::share::Service::ALL
        .iter()
        .zip(form.services)
    {
        let mark = if on { "[x]" } else { "[ ]" };
        items.push(ListItem::new(format!(
            "{mark} {}",
            crate::app::service_label(*s)
        )));
    }
    items.push(ListItem::new(Span::styled("Share now...", t.accent)));
    debug_assert_eq!(items.len(), crate::app::SHARE_ROWS);
    let [l, d] = Layout::vertical([Constraint::Min(5), Constraint::Length(7)]).areas(area);
    draw_list(f, &t, l, "SHARE THIS COMPUTER'S RESOURCES", items, list, "");
    details(
        f,
        &t,
        d,
        vec![
            Line::from(Span::styled(
                "Everything you share can be read by ANYONE on this network.",
                t.warn,
            )),
            Line::from({
                let (f_ok, f_no) = app.shared_counts("files");
                let (p_ok, p_no) = app.shared_counts("pages");
                format!(
                    "Shared folder: {f_ok} file(s) served, {f_no} private or not a file \
                     (not served). Pages: {p_ok} served, {p_no} not served. Only files \
                     everyone on this computer may read are served."
                )
            }),
            Line::from(Span::styled(
                "Only one network at a time. Nothing can be sent to this computer.",
                t.dim,
            )),
        ],
    );
}

fn draw_peer_files(
    f: &mut Frame,
    app: &App,
    area: Rect,
    peer: &resilience_core::share::Peer,
    rows: &[resilience_core::share::SharedFile],
    list: &mut ListState,
) {
    let t = Theme::new(app.theme);
    let items: Vec<ListItem> = rows
        .iter()
        .map(|r| {
            ListItem::new(format!(
                "{:<56.56} {:>10}",
                messages::clean_line(&r.name, 56),
                human_bytes(r.size)
            ))
        })
        .collect();
    let [l, d] = Layout::vertical([Constraint::Min(5), Constraint::Length(5)]).areas(area);
    draw_list(
        f,
        &t,
        l,
        "SHARED FOLDER OF A NEARBY COMPUTER",
        items,
        list,
        "This computer's shared folder is empty.",
    );
    details(
        f,
        &t,
        d,
        vec![
            Line::from(format!("From {}.", peer.display())),
            Line::from(Span::styled(
                "Files are not checked. A copy goes into your Received folder; read it \
                 with Find & view files.",
                t.dim,
            )),
        ],
    );
}
