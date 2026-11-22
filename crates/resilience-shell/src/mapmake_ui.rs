//! Maps & navigation → "Make or update a map" (ROADMAP Phase 5b, ADR 0016
//! §10).
//!
//! The rows are the `.osm.pbf` extracts found on open USB sticks, then
//! every region of the list the system ships (Geofabrik's), filtered by
//! the words typed. Enter runs `resilience-mapmaker make` in the
//! foreground: it asks Geofabrik for the size (the only network use before
//! the person agrees), says what the build needs and asks before
//! downloading or building anything. Maps made here live in `~/Maps` and
//! are listed with the other maps.

use std::path::PathBuf;

use crate::input::{KeyCode, KeyEvent, KeyModifiers};
use crate::list::ListState;
use resilience_core::mapmake::{self, MakerConfig, Region};
use resilience_core::packs::PackCategory;

use super::library::{ItemPlace, LibItem};
use super::{App, Effect, Level, RunRequest, View, clamp_selection, is_back, nav};

pub const MAKE_MAP_LABEL: &str = "Make a map";

/// At most this many regions are shown at once (the list has ~550).
const MAX_ROWS: usize = 600;

/// One row of the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MakeRow {
    /// An extract on an open stick.
    Stick { volume: String, path: PathBuf },
    /// A region of the list; `made`: the month of a map made here.
    Region {
        id: String,
        path: String,
        made: Option<String>,
    },
}

impl MakeRow {
    pub fn label(&self) -> String {
        match self {
            MakeRow::Stick { volume, path } => format!(
                "From the stick {volume}: {}",
                path.file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
            ),
            MakeRow::Region { path, made, .. } => match made {
                Some(m) => format!("{path}  (made here: data of {m})"),
                None => path.clone(),
            },
        }
    }
}

impl App {
    pub(super) fn mapmaker_config(&self) -> Option<MakerConfig> {
        MakerConfig::load(&self.paths.mapmaker_config).ok()
    }

    pub(super) fn maps_folder(&self) -> PathBuf {
        self.home.join(mapmake::MAPS_FOLDER)
    }

    /// The maps made here (`~/Maps/map-osm-ID_YYYY-MM.mbtiles`).
    pub(super) fn made_maps(&self) -> Vec<LibItem> {
        let regions = self
            .mapmaker_config()
            .and_then(|c| mapmake::load_regions(&c.regions).ok())
            .unwrap_or_default();
        let mut v: Vec<LibItem> = std::fs::read_dir(self.maps_folder())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let id = mapmake::made_id(&name)?;
                let meta = std::fs::symlink_metadata(e.path()).ok()?;
                if !meta.is_file() {
                    return None;
                }
                let month = name
                    .rsplit_once('_')?
                    .1
                    .trim_end_matches(".mbtiles")
                    .to_string();
                let title = regions
                    .iter()
                    .find(|r| r.id == id)
                    .map_or_else(|| id.to_string(), |r| r.name.clone());
                Some(LibItem {
                    title,
                    description: format!(
                        "Made on this computer from OpenStreetMap data of {month}, kept in \
                         the folder Maps of your home. To update it, make it again in \
                         \"Make or update a map\"."
                    ),
                    path: e.path(),
                    size: meta.len(),
                    category: Some(PackCategory::Maps),
                    language: None,
                    place: ItemPlace::Made,
                })
            })
            .collect();
        v.sort_by(|a, b| a.title.cmp(&b.title));
        v
    }

    /// (region id, month) of the maps made here: the folder only, cheap
    /// enough for every frame.
    fn made_months(&self) -> Vec<(String, String)> {
        std::fs::read_dir(self.maps_folder())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().into_string().ok()?;
                let id = mapmake::made_id(&n)?.to_string();
                let month = n
                    .rsplit_once('_')?
                    .1
                    .trim_end_matches(".mbtiles")
                    .to_string();
                Some((id, month))
            })
            .take(1000)
            .collect()
    }

    /// Extracts on the open sticks: `*.osm.pbf` at the top or in the
    /// Resilience folder of the stick.
    fn stick_extracts(&self) -> Vec<MakeRow> {
        let (volumes, _) = self.load_volumes();
        let mut rows = Vec::new();
        for v in volumes.iter().filter(|v| v.usable()) {
            let Some(m) = &v.mountpoint else { continue };
            for dir in [m.clone(), m.join(resilience_core::packs::VOLUME_FOLDER)] {
                let Ok(rd) = std::fs::read_dir(&dir) else {
                    continue;
                };
                let mut found: Vec<PathBuf> = rd
                    .flatten()
                    .filter(|e| {
                        e.file_name().to_str().is_some_and(|n| {
                            n.ends_with(".osm.pbf") && !n.starts_with('.') && n.len() <= 120
                        }) && e.file_type().is_ok_and(|t| t.is_file())
                    })
                    .map(|e| e.path())
                    .take(50)
                    .collect();
                found.sort();
                rows.extend(found.into_iter().map(|path| MakeRow::Stick {
                    volume: v.display_name(),
                    path,
                }));
            }
        }
        rows
    }

    pub fn make_rows(&self, regions: &[Region], sticks: &[MakeRow], query: &str) -> Vec<MakeRow> {
        let made = self.made_months();
        let mut rows: Vec<MakeRow> = sticks.to_vec();
        rows.extend(
            mapmake::search(regions, query)
                .into_iter()
                .take(MAX_ROWS)
                .map(|r| MakeRow::Region {
                    id: r.id.clone(),
                    path: mapmake::region_path(regions, &r.id),
                    made: made
                        .iter()
                        .find(|(i, _)| *i == r.id)
                        .map(|(_, m)| m.clone()),
                }),
        );
        rows
    }

    pub(super) fn open_make_map(&mut self) -> Option<View> {
        let cfg = match self.mapmaker_config() {
            Some(c) => c,
            None => {
                self.notify(
                    Level::Error,
                    "Map making is not available on this computer.",
                );
                return None;
            }
        };
        let regions = match mapmake::load_regions(&cfg.regions) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "region list unreadable");
                self.notify(Level::Error, "The list of regions cannot be read.");
                return None;
            }
        };
        let sticks = self.stick_extracts();
        let n = self.make_rows(&regions, &sticks, "").len();
        Some(View::MakeMap {
            regions,
            sticks,
            query: String::new(),
            list: ListState::default().with_selected((n > 0).then_some(0)),
        })
    }

    pub(super) fn make_map_key(&mut self, mut view: View, key: KeyEvent) -> (Option<View>, Effect) {
        let View::MakeMap {
            regions,
            sticks,
            query,
            list,
        } = &mut view
        else {
            return (Some(view), Effect::None);
        };
        let rows = self.make_rows(regions, sticks, query);
        match key.code {
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if query.chars().count() < 40 {
                    query.push(c);
                }
                let n = self.make_rows(regions, sticks, query).len();
                list.select((n > 0).then_some(0));
            }
            KeyCode::Backspace if !query.is_empty() => {
                query.pop();
                let n = self.make_rows(regions, sticks, query).len();
                list.select((n > 0).then_some(0));
            }
            KeyCode::F(5) => {
                *sticks = self.stick_extracts();
                let n = self.make_rows(regions, sticks, query).len();
                clamp_selection(list, n);
            }
            KeyCode::Enter => {
                if let Some(r) = list.selected().and_then(|i| rows.get(i)) {
                    return (Some(view), self.make_map_effect(r));
                }
            }
            _ if is_back(key) && key.code != KeyCode::Char('h') => return (None, Effect::None),
            _ => nav(list, rows.len(), key),
        }
        (Some(view), Effect::None)
    }

    fn make_map_effect(&mut self, row: &MakeRow) -> Effect {
        let Some(cfg) = self.mapmaker_config() else {
            return Effect::None;
        };
        let mut argv = vec![cfg.program.clone(), "make".to_string()];
        match row {
            MakeRow::Stick { path, .. } => {
                argv.push("--pbf".into());
                argv.push(path.to_string_lossy().into_owned());
            }
            MakeRow::Region { id, .. } => {
                argv.push("--region".into());
                argv.push(id.clone());
            }
        }
        tracing::info!("making a map");
        Effect::Run(RunRequest {
            label: MAKE_MAP_LABEL.into(),
            argv,
            extra_args: Vec::new(),
            env: Vec::new(),
            polkit_agent: None,
            clean_env: false,
            pause_after: true,
            banner: None,
        })
    }

    /// After the builder: every map list (rows are read from ~/Maps each time).
    pub(super) fn make_map_after_run(&mut self, label: &str) {
        if label == MAKE_MAP_LABEL {
            self.reload_packs();
        }
    }
}
