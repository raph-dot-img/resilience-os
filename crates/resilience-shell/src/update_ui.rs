//! Recovery → "Update from a USB stick" (ADR 0023 point 7).
//!
//! Lists removable media; on the opened ones, looks for
//! `resilience-update/resilience-update.toml` and shows what it claims
//! (edition, version) as a claim only. `c` runs `resilience-admin
//! check-update DIR` as the user (nothing imported); Enter, after a
//! summary, runs `pkexec resilience-admin apply-update DIR`, which checks
//! everything again as root. Shown only when the system has release keys.

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::input::{KeyCode, KeyEvent};
use crate::list::ListState;
use resilience_core::admin::AdminConfig;
use resilience_core::media::Volume;
use resilience_core::update::{self, Pointer, UpdateSettings};

use super::{
    App, Effect, Level, Modal, Pending, RunRequest, View, clamp_selection, is_activate, is_back,
    nav,
};

pub const UPDATE_CHECK_LABEL: &str = "Check an update";
pub const UPDATE_APPLY_LABEL: &str = "Install an update";

/// One removable volume on the update screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateRow {
    pub volume: Volume,
    /// The bundle folder, when the opened volume has one.
    pub bundle: Option<PathBuf>,
    /// What its pointer file says (not checked), or why it is unreadable.
    pub claim: Option<Result<Pointer, String>>,
}

impl UpdateRow {
    /// The bundle and its readable claim: what `c` and Enter act on.
    pub fn ready(&self) -> Option<(&Path, &Pointer)> {
        match (&self.bundle, &self.claim) {
            (Some(b), Some(Ok(p))) => Some((b.as_path(), p)),
            _ => None,
        }
    }
}

/// The pointer file, read as the user with a size bound.
fn read_claim(file: &Path) -> Result<Pointer, String> {
    let f = std::fs::File::open(file).map_err(|e| e.to_string())?;
    let mut s = String::new();
    f.take(update::MAX_TOML + 1)
        .read_to_string(&mut s)
        .map_err(|e| e.to_string())?;
    if s.len() as u64 > update::MAX_TOML {
        return Err("the file is too large".into());
    }
    update::parse_pointer(&s)
}

/// Look for an update on each opened volume.
pub fn update_rows(volumes: Vec<Volume>) -> Vec<UpdateRow> {
    volumes
        .into_iter()
        .map(|volume| {
            let dir = volume
                .mountpoint
                .as_ref()
                .filter(|_| volume.problem.is_none())
                .map(|m| m.join(update::BUNDLE_DIR));
            let pointer = dir.as_ref().map(|d| d.join(update::POINTER_FILE));
            let (bundle, claim) = match pointer {
                Some(p) if p.is_file() => (dir, Some(read_claim(&p))),
                _ => (None, None),
            };
            UpdateRow {
                volume,
                bundle,
                claim,
            }
        })
        .collect()
}

impl App {
    /// The update settings, when this system can take updates at all.
    pub fn update_settings(&self) -> Option<&UpdateSettings> {
        self.sys.updates.as_ref().filter(|u| u.usable())
    }

    pub(super) fn load_update_rows(&self) -> (Vec<UpdateRow>, Option<String>) {
        let (v, e) = self.load_volumes();
        (update_rows(v), e)
    }

    pub(super) fn open_updates(&mut self) -> Option<View> {
        self.update_settings()?;
        let (rows, error) = self.load_update_rows();
        let first = rows
            .iter()
            .position(|r| r.ready().is_some())
            .or((!rows.is_empty()).then_some(0));
        Some(View::Updates {
            rows,
            error,
            list: ListState::default().with_selected(first),
        })
    }

    pub(super) fn reload_updates(&mut self) {
        let (fresh, e) = self.load_update_rows();
        for v in &mut self.stack {
            if let View::Updates { rows, error, list } = v {
                *rows = fresh.clone();
                *error = e.clone();
                clamp_selection(list, rows.len());
            }
        }
    }

    /// Keys of the update screen (the view is popped while handled).
    pub(super) fn updates_key(&mut self, mut view: View, key: KeyEvent) -> (Option<View>, Effect) {
        let View::Updates { rows, error, list } = &mut view else {
            return (Some(view), Effect::None);
        };
        let selected = list.selected().and_then(|i| rows.get(i)).cloned();
        let mut effect = Effect::None;
        match key.code {
            KeyCode::Char('r') => {
                (*rows, *error) = self.load_update_rows();
                clamp_selection(list, rows.len());
            }
            KeyCode::Char('c') => {
                if let Some(r) = &selected {
                    effect = self.check_update(r);
                }
            }
            _ if is_activate(key) => match selected {
                Some(r) if r.volume.mountpoint.is_none() => {
                    if self.mount_volume(&r.volume).is_some() {
                        (*rows, *error) = self.load_update_rows();
                        clamp_selection(list, rows.len());
                        let found = rows
                            .iter()
                            .any(|x| x.volume.device == r.volume.device && x.bundle.is_some());
                        let name = r.volume.display_name();
                        if found {
                            self.notify(Level::Info, format!("{name} holds an update."));
                        } else {
                            self.notify(Level::Info, format!("No update on {name}."));
                        }
                    }
                }
                Some(r) => self.confirm_apply_update(&r),
                None => {}
            },
            _ if is_back(key) => return (None, Effect::None),
            _ => nav(list, rows.len(), key),
        }
        (Some(view), effect)
    }

    /// Explain why `r` cannot be checked or installed; true when it cannot.
    fn not_ready(&mut self, r: &UpdateRow) -> bool {
        let body = match (&r.bundle, &r.claim) {
            (Some(_), Some(Ok(_))) => return false,
            (Some(_), Some(Err(e))) => format!(
                "The update on '{}' cannot be read ({}): {e}.\n\nIt may be damaged or \
                 written by an incompatible version. Nothing was installed.",
                r.volume.display_name(),
                update::POINTER_FILE
            ),
            _ if r.volume.mountpoint.is_none() => format!(
                "'{}' is not open yet. Press Enter to open it.",
                r.volume.display_name()
            ),
            _ => format!(
                "There is no update on '{}'. An update is a folder named {} at the top \
                 of the stick.",
                r.volume.display_name(),
                update::BUNDLE_DIR
            ),
        };
        self.modal = Some(Modal::Message {
            title: "NO UPDATE HERE".into(),
            body,
            error: false,
        });
        true
    }

    fn admin_config_for(&mut self, what: &str) -> Option<AdminConfig> {
        match AdminConfig::read(&self.paths.admin_config) {
            Ok(c) => Some(c),
            Err(e) => {
                self.notify(Level::Error, format!("{what} is unavailable: {e}"));
                None
            }
        }
    }

    /// `c`: `resilience-admin check-update DIR`, as the user.
    fn check_update(&mut self, r: &UpdateRow) -> Effect {
        if self.not_ready(r) {
            return Effect::None;
        }
        let Some(cfg) = self.admin_config_for("checking an update") else {
            return Effect::None;
        };
        let dir = r.bundle.clone().unwrap_or_default();
        Effect::Run(RunRequest {
            label: UPDATE_CHECK_LABEL.into(),
            argv: vec![
                cfg.helper.to_string_lossy().into_owned(),
                "check-update".into(),
            ],
            extra_args: vec![dir],
            env: Vec::new(),
            polkit_agent: None,
            clean_env: true,
            pause_after: true,
            banner: Some("CHECKING THE UPDATE. Nothing is installed. Press Ctrl-C to stop.".into()),
        })
    }

    /// Enter on a stick with an update: what will happen, then ask.
    fn confirm_apply_update(&mut self, r: &UpdateRow) {
        if self.not_ready(r) {
            return;
        }
        let Some((dir, claim)) = r.ready() else {
            return;
        };
        let Some(running) = self
            .update_settings()
            .map(|u| (u.edition.clone(), u.version.clone()))
        else {
            return;
        };
        if dir.to_str().is_none() {
            self.modal = Some(Modal::Message {
                title: "NOT POSSIBLE".into(),
                body: "The stick's name contains characters that cannot be passed to the \
                       installer. Rename the stick (letters, digits, spaces) and try again."
                    .into(),
                error: true,
            });
            return;
        }
        if !self.has_administrator("Installing an update") {
            return;
        }
        let body = format!(
            "The stick says it holds Resilience {} {}. This computer runs {} {}.\n\n\
             Everything is checked again before anything is installed: the update must be \
             intact, signed by a Resilience release key, for this edition and newer than \
             the running version.\n\n\
             The new version is copied to this computer (several minutes; keep the stick \
             plugged in) and starts at the next restart. The running system is not \
             changed. Your files, accounts, messages and resource packs are kept. The \
             current version stays in the boot menu and in Recovery > System generations, \
             so you can go back.\n\n\
             An administrator must type their password next; the update is recorded in \
             the audit log.",
            claim.edition, claim.version, running.0, running.1
        );
        self.modal = Some(Modal::Confirm {
            title: "INSTALL AN UPDATE".into(),
            body,
            confirm: "Install".into(),
            action: Pending::ApplyUpdate {
                dir: dir.to_path_buf(),
            },
            yes_focused: false,
        });
    }

    /// The fixed argv: `pkexec <helper> apply-update DIR`.
    pub(super) fn apply_update_effect(&mut self, dir: &Path) -> Effect {
        let Some(cfg) = self.admin_config_for("installing an update") else {
            return Effect::None;
        };
        let Some(d) = dir.to_str() else {
            return Effect::None;
        };
        tracing::warn!(bundle = d, "offline update requested");
        Effect::Run(RunRequest {
            label: UPDATE_APPLY_LABEL.into(),
            argv: vec![
                cfg.pkexec.to_string_lossy().into_owned(),
                cfg.helper.to_string_lossy().into_owned(),
                "apply-update".into(),
                d.into(),
            ],
            extra_args: Vec::new(),
            env: vec![(
                "SHELL".into(),
                cfg.session_shell.to_string_lossy().into_owned(),
            )],
            polkit_agent: Some(cfg.pkttyagent.to_string_lossy().into_owned()),
            clean_env: false,
            pause_after: true,
            banner: Some(
                "INSTALL AN UPDATE. An administrator must now type their password. To \
                 cancel, press Ctrl-C while the password is asked."
                    .into(),
            ),
        })
    }
}
