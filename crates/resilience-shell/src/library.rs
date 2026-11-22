//! Offline library and resource packs (ADR 0013): the state behind the
//! "Offline library" and "Resource packs" screens, and the runs they start.
//!
//! The interface never installs anything itself. It shows the catalog and
//! what is installed, lets the user tick packs, and runs
//! `resilience-library change …` in the foreground, which asks for an
//! administrator's password and streams the files to the pack store.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::list::ListState;
use resilience_core::human_bytes;
use resilience_core::network::LinkLevel;
use resilience_core::packs::{self, Inventory, PackCategory, PackConfig, PackKind, PackState};

use super::{
    App, Effect, LIBRARY_LABEL, Level, MediaPurpose, Modal, PACKS_LABEL, Pending, RunRequest, View,
};

/// One line of the packs screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackRow {
    Header(PackCategory),
    Pack(usize),
    OthersHeader,
    Other(usize),
}

impl PackRow {
    pub fn selectable(self) -> bool {
        matches!(self, PackRow::Pack(_) | PackRow::Other(_))
    }
}

/// The packs screen: catalog, what is installed, and the user's ticks.
#[derive(Debug, Clone)]
pub struct PacksScreen {
    pub cfg: PackConfig,
    pub inv: Inventory,
    /// Catalog packs ticked: added if absent, removed if installed.
    pub marked: BTreeSet<usize>,
    /// Partial downloads ticked to be thrown away.
    pub discard: BTreeSet<usize>,
    /// Other files (older versions) ticked for removal.
    pub marked_other: BTreeSet<usize>,
    /// Only packs in this language are listed (None: all).
    pub language: Option<String>,
    pub free: Option<u64>,
}

/// What applying the ticks would do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PacksPlan {
    /// Catalog indices to install.
    pub get: Vec<usize>,
    /// Files to remove.
    pub remove: Vec<String>,
    /// Bytes still to receive for `get` (partial downloads count less).
    pub add_bytes: u64,
    pub remove_bytes: u64,
}

impl PacksScreen {
    pub fn load(path: &Path) -> Result<PacksScreen, String> {
        let cfg = PackConfig::load(path).map_err(|e| e.to_string())?;
        let inv = packs::inventory(&cfg);
        let free = packs::free_space(&cfg.dir);
        Ok(PacksScreen {
            cfg,
            inv,
            marked: BTreeSet::new(),
            discard: BTreeSet::new(),
            marked_other: BTreeSet::new(),
            language: None,
            free,
        })
    }

    pub fn refresh(&mut self) {
        self.inv = packs::inventory(&self.cfg);
        self.free = packs::free_space(&self.cfg.dir);
        self.marked.clear();
        self.discard.clear();
        self.marked_other.clear();
    }

    /// Catalog packs grouped by category (catalog order inside a group),
    /// filtered by language, then files that are not in the catalog.
    pub fn rows(&self) -> Vec<PackRow> {
        let mut idx: Vec<usize> = (0..self.cfg.packs.len())
            .filter(|&i| {
                self.language
                    .as_ref()
                    .is_none_or(|l| &self.cfg.packs[i].language == l)
            })
            .collect();
        idx.sort_by_key(|&i| (self.cfg.packs[i].category, i));
        let mut rows = Vec::new();
        let mut last = None;
        for i in idx {
            let c = self.cfg.packs[i].category;
            if last != Some(c) {
                rows.push(PackRow::Header(c));
                last = Some(c);
            }
            rows.push(PackRow::Pack(i));
        }
        if !self.inv.others.is_empty() {
            rows.push(PackRow::OthersHeader);
            rows.extend((0..self.inv.others.len()).map(PackRow::Other));
        }
        rows
    }

    /// Space: untick/tick. A partial download cycles through continue
    /// (`[+]`), throw away (`[-]`) and nothing.
    pub fn toggle(&mut self, row: PackRow) {
        if let PackRow::Pack(i) = row
            && matches!(self.inv.states[i], PackState::Partial(_))
        {
            if self.marked.remove(&i) {
                self.discard.insert(i);
            } else if !self.discard.remove(&i) {
                self.marked.insert(i);
            }
            return;
        }
        let set = match row {
            PackRow::Pack(i) => (&mut self.marked, i),
            PackRow::Other(i) => (&mut self.marked_other, i),
            _ => return,
        };
        if !set.0.remove(&set.1) {
            set.0.insert(set.1);
        }
    }

    pub fn plan(&self) -> PacksPlan {
        let mut plan = PacksPlan::default();
        for &i in &self.marked {
            let p = &self.cfg.packs[i];
            match self.inv.states[i] {
                PackState::Installed => {
                    plan.remove.push(p.file.clone());
                    plan.remove_bytes += p.size;
                }
                PackState::Partial(n) => {
                    plan.get.push(i);
                    plan.add_bytes += p.size - n;
                }
                PackState::Absent => {
                    plan.get.push(i);
                    plan.add_bytes += p.size;
                }
            }
        }
        for &i in &self.discard {
            let p = &self.cfg.packs[i];
            if let PackState::Partial(n) = self.inv.states[i] {
                plan.remove.push(p.file.clone());
                plan.remove_bytes += n;
            }
        }
        for &j in &self.marked_other {
            let o = &self.inv.others[j];
            plan.remove.push(o.file.clone());
            plan.remove_bytes += o.size;
        }
        plan
    }

    /// Cycle the language filter: all, then each language of the catalog.
    pub fn next_language(&mut self) {
        let langs = self.cfg.languages();
        self.language = match &self.language {
            None => langs.first().cloned(),
            Some(l) => langs
                .iter()
                .position(|x| x == l)
                .and_then(|i| langs.get(i + 1))
                .cloned(),
        };
    }

    /// Text of the checkbox and the state column for a row.
    pub fn row_state(&self, row: PackRow) -> (&'static str, String) {
        match row {
            PackRow::Pack(i) => {
                let marked = self.marked.contains(&i);
                match (self.inv.states[i], marked) {
                    (PackState::Installed, false) => ("[*]", "installed".into()),
                    (PackState::Installed, true) => ("[-]", "remove".into()),
                    (PackState::Partial(_), _) if self.discard.contains(&i) => {
                        ("[-]", "discard".into())
                    }
                    (PackState::Partial(n), m) => (
                        if m { "[+]" } else { "[ ]" },
                        format!("{}% done", n * 100 / self.cfg.packs[i].size.max(1)),
                    ),
                    (PackState::Absent, true) => ("[+]", "add".into()),
                    (PackState::Absent, false) => ("[ ]", String::new()),
                }
            }
            PackRow::Other(j) => {
                if self.marked_other.contains(&j) {
                    ("[-]", "remove".into())
                } else {
                    ("[*]", "old version".into())
                }
            }
            _ => ("", String::new()),
        }
    }
}

/// Where a readable pack is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemPlace {
    /// Part of the system (Nix store).
    Builtin,
    /// Installed and checked.
    Installed,
    /// An older version or a file outside the catalog, in the packs directory.
    Other,
    /// On a mounted stick, not checked. `known`: name and size are in the catalog.
    Stick { volume: String, known: bool },
}

/// A file the library can open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibItem {
    pub title: String,
    pub description: String,
    pub path: PathBuf,
    pub size: u64,
    pub category: Option<PackCategory>,
    pub language: Option<String>,
    pub place: ItemPlace,
}

impl LibItem {
    /// Short tag for the list.
    pub fn tag(&self) -> String {
        match &self.place {
            ItemPlace::Builtin => "Built in".into(),
            ItemPlace::Stick { .. } => "USB stick".into(),
            ItemPlace::Other => "Other".into(),
            ItemPlace::Installed => self.category.map_or("Other", |c| c.label()).into(),
        }
    }
}

/// The map viewer opens at most this many files (`resilience-maps view`).
pub const MAX_MAPS: usize = 8;
const MAP_LABEL: &str = "Map";

/// Whether a pack file is of this kind (by its name).
fn is_kind(path: &Path, kind: PackKind) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .and_then(PackKind::of)
        == Some(kind)
}

/// Pack files of one kind a stick offers: catalog files (exact name and
/// size) and any other file of that kind in its Resilience-packs folder.
/// The Offline library asks for [`PackKind::Zim`]: map packs are not read in
/// the text browser.
pub fn stick_items(cfg: &PackConfig, volume: &str, mount: &Path, kind: PackKind) -> Vec<LibItem> {
    let mut items: Vec<LibItem> = packs::find_on_volume(cfg, mount)
        .into_iter()
        .filter(|(_, path)| is_kind(path, kind))
        .map(|(i, path)| {
            let p = &cfg.packs[i];
            LibItem {
                title: p.title.clone(),
                description: format!(
                    "On the stick {volume}, not installed and not checked yet: c checks \
                     its fingerprint. {}",
                    p.description
                ),
                path,
                size: p.size,
                category: Some(p.category),
                language: Some(p.language.clone()),
                place: ItemPlace::Stick {
                    volume: volume.into(),
                    known: true,
                },
            }
        })
        .collect();
    let folder = mount.join(packs::VOLUME_FOLDER);
    if let Ok(rd) = std::fs::read_dir(&folder) {
        let mut extra: Vec<LibItem> = rd
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let meta = std::fs::symlink_metadata(e.path()).ok()?;
                let listed = items.iter().any(|i| i.path == e.path());
                let wanted = packs::valid_file_name(&name) && is_kind(&e.path(), kind);
                (wanted && meta.is_file() && !listed).then(|| LibItem {
                    title: name.clone(),
                    description: format!(
                        "On the stick {volume}: a file that is not in this system's catalog. \
                         Its content cannot be checked; open it only if you trust where it \
                         comes from."
                    ),
                    path: e.path(),
                    size: meta.len(),
                    category: None,
                    language: None,
                    place: ItemPlace::Stick {
                        volume: volume.into(),
                        known: false,
                    },
                })
            })
            .collect();
        extra.sort_by(|a, b| a.title.cmp(&b.title));
        items.extend(extra);
    }
    items
}

/// Built-in packs, installed catalog packs (by category, then title), then
/// other files of the packs directory: only those of one kind.
pub fn library_items(cfg: &PackConfig, kind: PackKind) -> Vec<LibItem> {
    let inv = packs::inventory(cfg);
    let mut items: Vec<LibItem> = cfg
        .builtin
        .iter()
        .filter(|b| b.path.is_file() && is_kind(&b.path, kind))
        .map(|b| LibItem {
            title: b.title.clone(),
            description: b.description.clone(),
            path: b.path.clone(),
            size: std::fs::metadata(&b.path).map_or(0, |m| m.len()),
            category: Some(b.category),
            language: Some(b.language.clone()),
            place: ItemPlace::Builtin,
        })
        .collect();
    let mut installed: Vec<LibItem> = cfg
        .packs
        .iter()
        .zip(&inv.states)
        .filter(|(p, s)| **s == PackState::Installed && p.kind() == Some(kind))
        .map(|(p, _)| LibItem {
            title: p.title.clone(),
            description: p.description.clone(),
            path: cfg.installed_path(p),
            size: p.size,
            category: Some(p.category),
            language: Some(p.language.clone()),
            place: ItemPlace::Installed,
        })
        .collect();
    installed.sort_by(|a, b| (a.category, &a.title).cmp(&(b.category, &b.title)));
    items.extend(installed);
    let others = inv
        .others
        .iter()
        .filter(|o| PackKind::of(&o.file) == Some(kind));
    items.extend(others.map(|o| {
        let older = o.older_of.map(|i| cfg.packs[i].title.as_str());
        LibItem {
            title: older.map_or_else(|| o.file.clone(), |t| format!("{t} (older version)")),
            description: format!("{}: not in the current catalog.", o.file),
            path: cfg.dir.join(&o.file),
            size: o.size,
            category: None,
            language: None,
            place: ItemPlace::Other,
        }
    }));
    items
}

/// Move the selection over selectable rows only, with wrap-around.
pub fn nav_rows(list: &mut ListState, rows: &[PackRow], key: crate::input::KeyEvent) {
    use crate::input::KeyCode;
    let selectable: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.selectable())
        .map(|(i, _)| i)
        .collect();
    if selectable.is_empty() {
        list.select(None);
        return;
    }
    let cur = list
        .selected()
        .and_then(|s| selectable.iter().position(|&i| i >= s))
        .unwrap_or(0);
    let n = selectable.len();
    let next = match key.code {
        KeyCode::Up | KeyCode::Char('k') => (cur + n - 1) % n,
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => (cur + 1) % n,
        KeyCode::Home | KeyCode::Char('g') => 0,
        KeyCode::End | KeyCode::Char('G') => n - 1,
        KeyCode::PageUp => cur.saturating_sub(10),
        KeyCode::PageDown => (cur + 10).min(n - 1),
        _ => cur,
    };
    list.select(Some(selectable[next]));
}

/// Select the first selectable row at or after the current selection.
pub fn fix_selection(list: &mut ListState, rows: &[PackRow]) {
    let from = list.selected().unwrap_or(0);
    let pick = rows
        .iter()
        .enumerate()
        .skip(from)
        .chain(rows.iter().enumerate())
        .find(|(_, r)| r.selectable())
        .map(|(i, _)| i);
    list.select(pick);
}

impl App {
    fn packs_config_or_explain(&mut self) -> Option<PackConfig> {
        match PackConfig::load(&self.paths.packs_config) {
            Ok(c) => Some(c),
            Err(e) => {
                self.notify(Level::Error, format!("resource packs are unavailable: {e}"));
                self.modal = Some(Modal::Message {
                    title: "RESOURCE PACKS UNAVAILABLE".into(),
                    body: format!("The list of resource packs could not be read:\n\n{e}"),
                    error: true,
                });
                None
            }
        }
    }

    /// Pack files of one kind, including those on mounted sticks.
    pub(super) fn all_library_items(&self, cfg: &PackConfig, kind: PackKind) -> Vec<LibItem> {
        let mut items = library_items(cfg, kind);
        let (volumes, _) = self.load_volumes();
        for v in volumes.iter().filter(|v| v.usable()) {
            if let Some(m) = &v.mountpoint {
                items.extend(stick_items(cfg, &v.display_name(), m, kind));
            }
        }
        items
    }

    pub(super) fn open_library(&mut self) -> Option<View> {
        let cfg = self.packs_config_or_explain()?;
        let items = self.all_library_items(&cfg, PackKind::Zim);
        let sel = (!items.is_empty()).then_some(0);
        Some(View::Library {
            kind: PackKind::Zim,
            items,
            list: ListState::default().with_selected(sel),
        })
    }

    /// Maps & navigation: the maps of this computer and of open sticks.
    pub(super) fn open_map_files(&mut self) -> Option<View> {
        let cfg = self.packs_config_or_explain()?;
        let items = self.all_library_items(&cfg, PackKind::Map);
        let sel = (!items.is_empty()).then_some(0);
        Some(View::Library {
            kind: PackKind::Map,
            items,
            list: ListState::default().with_selected(sel),
        })
    }

    pub(super) fn open_packs(&mut self) -> Option<View> {
        let s = match PacksScreen::load(&self.paths.packs_config) {
            Ok(s) => s,
            Err(e) => {
                self.packs_config_or_explain();
                tracing::warn!(error = %e, "packs screen unavailable");
                return None;
            }
        };
        let rows = s.rows();
        let mut list = ListState::default().with_selected(Some(0));
        fix_selection(&mut list, &rows);
        Some(View::Packs {
            s: Box::new(s),
            list,
        })
    }

    /// Refresh every library and packs screen after a change.
    pub(super) fn reload_packs(&mut self) {
        let cfg = PackConfig::load(&self.paths.packs_config).ok();
        let fresh = cfg.as_ref().map(|c| {
            (
                self.all_library_items(c, PackKind::Zim),
                self.all_library_items(c, PackKind::Map),
            )
        });
        for v in &mut self.stack {
            if let (View::Library { kind, items, list }, Some((zim, maps))) = (&mut *v, &fresh) {
                *items = match kind {
                    PackKind::Zim => zim.clone(),
                    PackKind::Map => maps.clone(),
                };
                super::clamp_selection(list, items.len());
            }
            match v {
                View::Packs { s, list } => {
                    s.refresh();
                    let rows = s.rows();
                    fix_selection(list, &rows);
                }
                View::Library { .. } => {}
                _ => {}
            }
        }
    }

    /// `u` in the library: open a stick to read the packs on it.
    pub(super) fn choose_reading_stick(&mut self) -> View {
        let (volumes, error) = self.load_volumes();
        let sel = (!volumes.is_empty()).then_some(0);
        View::Media {
            volumes,
            error,
            list: ListState::default().with_selected(sel),
            purpose: MediaPurpose::ReadPacks,
        }
    }

    /// `c` in the library: compare a file on a stick with the catalog.
    pub(super) fn check_pack_effect(&mut self, item: &LibItem) -> Effect {
        if !matches!(item.place, ItemPlace::Stick { .. }) {
            self.modal = Some(Modal::Message {
                title: "ALREADY CHECKED".into(),
                body: "Installed packs were checked against their fingerprint when they were \
                       installed, and built-in packs are part of the system. Only packs on a \
                       USB stick need checking."
                    .into(),
                error: false,
            });
            return Effect::None;
        }
        Effect::Run(RunRequest {
            label: LIBRARY_LABEL.into(),
            argv: vec![
                self.sys.tools.library.clone(),
                "check".into(),
                item.path.to_string_lossy().into_owned(),
            ],
            extra_args: Vec::new(),
            env: Vec::new(),
            polkit_agent: None,
            clean_env: false,
            pause_after: true,
            banner: Some(format!(
                "CHECKING {}. Large packs take a while; Ctrl-C stops.",
                item.title
            )),
        })
    }

    /// Suggested sets of the packs screen just below the top of the stack.
    fn packs_below(&mut self) -> Option<&mut PacksScreen> {
        self.stack.iter_mut().rev().find_map(|v| match v {
            View::Packs { s, .. } => Some(&mut **s),
            _ => None,
        })
    }

    pub(super) fn pack_sets_len(&mut self) -> usize {
        self.packs_below().map_or(0, |s| s.cfg.sets.len())
    }

    /// Tick every pack of set `i` that is not installed yet.
    pub(super) fn apply_pack_set(&mut self, i: usize) {
        let Some(s) = self.packs_below() else { return };
        let Some(set) = s.cfg.sets.get(i).cloned() else {
            return;
        };
        let mut n = 0;
        for id in &set.packs {
            if let Some(j) = s.cfg.packs.iter().position(|p| &p.id == id)
                && s.inv.states[j] != PackState::Installed
            {
                s.discard.remove(&j);
                n += usize::from(s.marked.insert(j));
            }
        }
        let bytes = s.plan().add_bytes;
        self.notify(
            Level::Info,
            format!(
                "{}: {n} more packs ticked ({} to add in total). Press d or u to get them.",
                set.title,
                human_bytes(bytes)
            ),
        );
    }

    /// Open the map viewer (`resilience-maps view`, in the foreground, as the
    /// user). Without `item`: on the built-in and installed maps. With one:
    /// on that map (and the built-in overview beneath it), then the other
    /// installed maps; the built-in overview alone when it is the item.
    /// At most [`MAX_MAPS`] files, as the viewer accepts.
    pub(super) fn open_map_effect(&mut self, item: Option<&LibItem>) -> Effect {
        let Some(cfg) = self.packs_config_or_explain() else {
            return Effect::None;
        };
        let known = library_items(&cfg, PackKind::Map);
        let builtin = known.iter().filter(|i| i.place == ItemPlace::Builtin);
        let installed = known.iter().filter(|i| i.place == ItemPlace::Installed);
        let mut files: Vec<PathBuf> = builtin.map(|i| i.path.clone()).collect();
        match item {
            Some(i) if i.place == ItemPlace::Builtin => {}
            Some(i) => {
                files.push(i.path.clone());
                files.extend(installed.map(|i| i.path.clone()));
            }
            None => files.extend(installed.map(|i| i.path.clone())),
        }
        let mut seen = BTreeSet::new();
        files.retain(|f| seen.insert(f.clone()));
        files.truncate(MAX_MAPS);
        if files.is_empty() {
            self.modal = Some(Modal::Message {
                title: "NO MAP".into(),
                body: "This computer has no map yet. Map packs are listed in Resource \
                       packs (Knowledge section): tick one and download it, or copy it \
                       from a USB stick."
                    .into(),
                error: false,
            });
            return Effect::None;
        }
        let mut argv = vec![self.sys.tools.maps.clone(), "view".to_string()];
        for f in &files {
            argv.extend(["--pack".to_string(), f.to_string_lossy().into_owned()]);
        }
        tracing::info!(maps = files.len(), "opening the map");
        Effect::Run(RunRequest {
            label: MAP_LABEL.into(),
            argv,
            extra_args: Vec::new(),
            env: Vec::new(),
            polkit_agent: None,
            clean_env: false,
            pause_after: false,
            banner: None,
        })
    }

    /// Open a pack in the text browser (through `resilience-library open`).
    pub(super) fn open_pack_effect(&self, item: &LibItem, search: Option<&str>) -> Effect {
        let mut argv = vec![self.sys.tools.library.clone(), "open".into()];
        argv.push(item.path.to_string_lossy().into_owned());
        if let Some(q) = search {
            argv.extend(["--search".to_string(), q.to_string()]);
        }
        tracing::info!(file = %item.path.display(), searching = search.is_some(), "opening a pack");
        Effect::Run(RunRequest {
            label: LIBRARY_LABEL.into(),
            argv,
            extra_args: Vec::new(),
            env: Vec::new(),
            polkit_agent: None,
            clean_env: false,
            pause_after: false,
            banner: None,
        })
    }

    /// Enter on the packs screen: everything known about a pack, in a
    /// scrolling view so that no part of it (a map's coverage text above
    /// all) can be cut off on a small screen.
    pub(super) fn pack_details(&self, s: &PacksScreen, row: PackRow) -> Option<View> {
        let text = match row {
            PackRow::Pack(i) => {
                let p = &s.cfg.packs[i];
                let state = match s.inv.states[i] {
                    PackState::Installed => "installed".to_string(),
                    PackState::Partial(n) => format!(
                        "{} of {} downloaded; it continues where it stopped",
                        human_bytes(n),
                        human_bytes(p.size)
                    ),
                    PackState::Absent => "not installed".into(),
                };
                let mut text = format!("{}\n\n{}\n", p.title, p.description);
                // What a map covers comes first: it says what is missing.
                let facts = [
                    ("Coverage", &p.coverage),
                    ("Data date", &p.data_date),
                    ("Attribution", &p.attribution),
                ];
                if facts.iter().any(|(_, v)| v.is_some()) {
                    text.push('\n');
                }
                for (label, value) in facts {
                    if let Some(v) = value {
                        text.push_str(&format!("{label}: {}\n", v.trim()));
                    }
                }
                text.push_str(&format!(
                    "\nSize: {}\nLanguage: {}\nState: {state}\nLicense: {}\nSource: {}\n\
                     File: {}\nSHA-256: {}",
                    human_bytes(p.size),
                    p.language,
                    p.license,
                    p.source,
                    p.file,
                    p.sha256
                ));
                if let Some(n) = &p.notice {
                    text.push_str(&format!("\n\n[!!] {}", n.trim()));
                }
                if p.kind() == Some(PackKind::Map) {
                    text.push_str(
                        "\n\nThis is a map pack: it is opened from Maps & navigation, not in \
                         the Offline library.",
                    );
                }
                if p.urls.is_empty() {
                    text.push_str("\n\nThis pack cannot be downloaded: copy it from a USB stick.");
                }
                text
            }
            PackRow::Other(j) => {
                let o = &s.inv.others[j];
                let place = match PackKind::of(&o.file) {
                    Some(PackKind::Map) => "opened in Maps & navigation",
                    _ => "read in the Offline library",
                };
                match o.older_of {
                    Some(i) => format!(
                        "{}\n\nAn older version of '{}', replaced in the catalog by a newer \
                         one. It can still be {place}. Tick it and apply to free {}.",
                        o.file,
                        s.cfg.packs[i].title,
                        human_bytes(o.size)
                    ),
                    None => format!(
                        "{}\n\nThis file is not in the catalog of this system. It can still be \
                         {place}, or removed to free {}.",
                        o.file,
                        human_bytes(o.size)
                    ),
                }
            }
            _ => return None,
        };
        Some(View::Pager {
            title: "PACK DETAILS".into(),
            text,
            scroll: 0,
        })
    }

    /// `d`: download the ticked packs (and remove the ticked ones).
    pub(super) fn confirm_download(&mut self, s: &PacksScreen) {
        let plan = s.plan();
        if plan.get.is_empty() && plan.remove.is_empty() {
            self.modal = Some(Modal::Message {
                title: "NOTHING SELECTED".into(),
                body: "Move to a pack and press Space to tick it: [+] will be added, [-] \
                       will be removed. Then press d to download, or u to copy from a USB \
                       stick."
                    .into(),
                error: false,
            });
            return;
        }
        let no_url: Vec<&str> = plan
            .get
            .iter()
            .filter(|&&i| s.cfg.packs[i].urls.is_empty())
            .map(|&i| s.cfg.packs[i].title.as_str())
            .collect();
        if !no_url.is_empty() {
            self.modal = Some(Modal::Message {
                title: "NOT DOWNLOADABLE".into(),
                body: format!(
                    "These packs can only be copied from a USB stick (press u):\n\n{}",
                    no_url.join("\n")
                ),
                error: true,
            });
            return;
        }
        if !plan.get.is_empty() && self.net.level != LinkLevel::Routed {
            self.modal = Some(Modal::Message {
                title: "NO INTERNET CONNECTION".into(),
                body: "This computer has no route to the Internet, so nothing can be \
                       downloaded. Connect it (Networking > Wi-Fi networks, or a cable), or \
                       copy the packs from a USB stick with u."
                    .into(),
                error: true,
            });
            return;
        }
        if !self.has_administrator("Installing resource packs") {
            return;
        }
        self.confirm_change(s, &plan, None, &[]);
    }

    /// `u`: choose a stick to copy the ticked packs (or all it has) from.
    pub(super) fn choose_pack_stick(&mut self) -> Option<View> {
        if !self.has_administrator("Installing resource packs") {
            return None;
        }
        let (volumes, error) = self.load_volumes();
        let sel = (!volumes.is_empty()).then_some(0);
        Some(View::Media {
            volumes,
            error,
            list: ListState::default().with_selected(sel),
            purpose: MediaPurpose::InstallPacks,
        })
    }

    /// A stick was opened from the packs screen: install from it.
    pub fn packs_from_mount(&mut self, mount: &Path) {
        let Some(s) = self.stack.iter().rev().find_map(|v| match v {
            View::Packs { s, .. } => Some((**s).clone()),
            _ => None,
        }) else {
            return;
        };
        let found = packs::find_on_volume(&s.cfg, mount);
        let mut plan = s.plan();
        let mut missing = Vec::new();
        if plan.get.is_empty() {
            // Nothing ticked: offer every pack on the stick that is not installed.
            for (i, _) in &found {
                if s.inv.states[*i] != PackState::Installed {
                    plan.get.push(*i);
                }
            }
        } else {
            plan.get.retain(|i| {
                let on = found.iter().any(|(f, _)| f == i);
                if !on {
                    missing.push(s.cfg.packs[*i].title.clone());
                }
                on
            });
        }
        plan.add_bytes = plan
            .get
            .iter()
            .map(|&i| {
                let p = &s.cfg.packs[i];
                match s.inv.states[i] {
                    PackState::Partial(n) => p.size - n,
                    _ => p.size,
                }
            })
            .sum();
        if plan.get.is_empty() {
            self.modal = Some(Modal::Message {
                title: "NO PACKS ON THIS STICK".into(),
                body: format!(
                    "No pack {}was found on this stick, at its top level or in a folder \
                     named {}. A pack is recognised by its exact file name and size (press \
                     Enter on a pack for its file name).",
                    if missing.is_empty() {
                        "of the catalog "
                    } else {
                        "you ticked "
                    },
                    packs::VOLUME_FOLDER
                ),
                error: true,
            });
            return;
        }
        self.confirm_change(&s, &plan, Some(mount), &missing);
    }

    fn confirm_change(
        &mut self,
        s: &PacksScreen,
        plan: &PacksPlan,
        from: Option<&Path>,
        missing: &[String],
    ) {
        let mut body = String::new();
        if !plan.get.is_empty() {
            body.push_str(match from {
                Some(_) => "Copy from the USB stick:\n",
                None => "Download:\n",
            });
            for &i in &plan.get {
                let p = &s.cfg.packs[i];
                body.push_str(&format!("  + {} ({})\n", p.title, human_bytes(p.size)));
            }
        }
        if !plan.remove.is_empty() {
            body.push_str("Remove (for every account):\n");
            for f in &plan.remove {
                let title = s
                    .cfg
                    .packs
                    .iter()
                    .find(|p| &p.file == f)
                    .map_or(f.as_str(), |p| p.title.as_str());
                body.push_str(&format!("  - {title}\n"));
            }
        }
        if !missing.is_empty() {
            body.push_str(&format!("Not on this stick: {}\n", missing.join(", ")));
        }
        body.push('\n');
        if plan.add_bytes > 0 {
            body.push_str(&format!("{} to add", human_bytes(plan.add_bytes)));
            if let Some(f) = s.free {
                body.push_str(&format!(", {} free", human_bytes(f + plan.remove_bytes)));
                if plan.add_bytes > f + plan.remove_bytes {
                    body.push_str(". [!!] NOT ENOUGH SPACE: some packs will be refused");
                }
            }
            body.push_str(".\n");
        }
        if from.is_none() && plan.add_bytes > 0 {
            body.push_str(
                "Large downloads can take hours. Ctrl-C stops; it continues where it \
                 stopped next time.\n",
            );
        }
        body.push_str(
            "Every file is checked against its fingerprint before it is installed. An \
             administrator must type their password next.",
        );
        self.modal = Some(Modal::Confirm {
            title: "RESOURCE PACKS".into(),
            body,
            confirm: "Continue".into(),
            action: Pending::ChangePacks {
                get: plan
                    .get
                    .iter()
                    .map(|&i| s.cfg.packs[i].id.clone())
                    .collect(),
                remove: plan.remove.clone(),
                from: from.map(Path::to_path_buf),
            },
            yes_focused: false,
        });
    }

    /// The fixed argv: `resilience-library change [--from M] [--remove F]… [--get ID]…`.
    pub(super) fn change_packs_effect(
        &mut self,
        get: &[String],
        remove: &[String],
        from: Option<&Path>,
    ) -> Effect {
        let mut argv = vec![self.sys.tools.library.clone(), "change".into()];
        if let Some(m) = from {
            argv.extend(["--from".to_string(), m.to_string_lossy().into_owned()]);
        }
        for f in remove {
            argv.extend(["--remove".to_string(), f.clone()]);
        }
        for id in get {
            argv.extend(["--get".to_string(), id.clone()]);
        }
        tracing::warn!(?get, ?remove, from = ?from, "resource pack change requested");
        Effect::Run(RunRequest {
            label: PACKS_LABEL.into(),
            argv,
            extra_args: Vec::new(),
            env: Vec::new(),
            polkit_agent: None,
            clean_env: false,
            pause_after: true,
            banner: Some(
                "RESOURCE PACKS. An administrator must now type their password. Ctrl-C \
                 stops at any time; an unfinished download continues next time."
                    .into(),
            ),
        })
    }
}
